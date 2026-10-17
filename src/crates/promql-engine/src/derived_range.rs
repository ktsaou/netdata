// SPDX-License-Identifier: GPL-3.0-or-later
// Subquery range continuations derive from pinned promql/engine.go; see NOTICE.
use crate::{
    accounting::{MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    derived_matrix::{self, Reader, Window},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions, RowFilter, drop_reserved},
    evaluator_roster::Descriptor,
    frontend::ast::NodeId,
    mad,
    query_scopes::Grid,
    range_float::{RateInput, RateWindow},
    range_numeric::{NumericRangeFunction, NumericStep},
    range_rates,
    range_reduce::{FoldError, FoldKind, FoldStep, RangeFunction},
    result::OwnedValue,
    storage::{Control, Interrupted},
    temporal_quantile,
    value::SampleRef,
};

#[derive(Clone, Copy)]
pub(crate) struct Input {
    pub(crate) body: NodeId,
    pub(crate) grid: Grid,
    pub(crate) options: EvaluationOptions,
    pub(crate) function: RangeFunction,
    pub(crate) parameters: [f64; 2],
    pub(crate) range_ns: i64,
    pub(crate) offset_ns: i64,
    pub(crate) fixed: bool,
    pub(crate) scheduled: bool,
}
impl Input {
    fn window(self) -> Window {
        let first_end_ms = self
            .options
            .start_ms
            .wrapping_sub(self.offset_ns / 1_000_000);
        Window {
            first_end_ms,
            end_ms: if self.fixed {
                first_end_ms
            } else {
                self.options
                    .time_ms
                    .wrapping_sub(self.offset_ns / 1_000_000)
            },
            range_ms: self.range_ns / 1_000_000,
            step_ms: self.options.step_ms,
        }
    }
    fn rate_input(self) -> RateInput {
        let window = self.window();
        RateInput {
            window: RateWindow {
                start_ms: window.end_ms.wrapping_sub(window.range_ms),
                end_ms: window.end_ms,
                range_ns: self.range_ns,
                start_timestamps: self.options.start_timestamps,
            },
            range_start_ms: self
                .options
                .time_ms
                .wrapping_sub(self.range_ns.wrapping_add(self.offset_ns) / 1_000_000),
            range_end_ms: self
                .options
                .time_ms
                .wrapping_sub(self.offset_ns / 1_000_000),
            data_bounds: None,
        }
    }
    fn drops_name(self) -> bool {
        match self.function {
            RangeFunction::Fold(kind) => !kind.preserves_name(),
            RangeFunction::Rate(_) | RangeFunction::Numeric(_) => true,
        }
    }
}

#[derive(Default)]
struct Output {
    float_samples: Option<crate::statistics::OwnedSamples>,
    value: Option<OwnedValue>,
    invalid_quantile: bool,
    annotations: [Option<AnnotationKind>; 6],
    nonempty: bool,
}
enum Pass {
    Replay,
    Complete(Output),
    Backing(temporal_quantile::Backing),
}
enum Kernel {
    Fold(FoldStep),
    Rate(range_rates::Step),
    Numeric(Option<NumericStep>),
    Mad(mad::Step),
    Quantile(temporal_quantile::Step),
}
impl Kernel {
    fn new(input: Input, account: &QueryAccounting) -> Self {
        match input.function {
            RangeFunction::Fold(kind) => {
                Self::Fold(FoldStep::new(kind, input.options.start_timestamps))
            }
            RangeFunction::Rate(kind) => {
                Self::Rate(range_rates::Step::new(input.rate_input(), kind))
            }
            RangeFunction::Numeric(NumericRangeFunction::Mad) => Self::Mad(mad::Step::default()),
            RangeFunction::Numeric(NumericRangeFunction::Quantile) => {
                Self::Quantile(temporal_quantile::Step::new(input.parameters[0], account))
            }
            RangeFunction::Numeric(kind) => Self::Numeric(Some(NumericStep::new(
                kind,
                input.parameters,
                input.options.time_ms,
                account,
            ))),
        }
    }
    fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        match self {
            Self::Fold(step) => step.receive(sample, control, account).map_err(fold_error),
            Self::Rate(step) => step.receive(sample, control, account).map_err(Into::into),
            Self::Numeric(step) => step
                .as_mut()
                .expect("numeric pass")
                .receive(sample, control, account)
                .map_err(Into::into),
            Self::Mad(step) => step.receive(sample, control, account).map_err(Into::into),
            Self::Quantile(step) => step.receive(sample, control, account).map_err(Into::into),
        }
    }
    fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Pass, EvaluationError> {
        let mut output = Output::default();
        match self {
            Self::Fold(step) => {
                let Some(result) = step.finish_pass(control, account).map_err(fold_error)? else {
                    return Ok(Pass::Replay);
                };
                output.float_samples = result.float_samples;
                output.value = result.value;
                output.annotations[..2].copy_from_slice(&result.annotations);
                output.nonempty = result.nonempty;
            }
            Self::Rate(step) => {
                let Some(result) = step.finish_pass(control, account)? else {
                    return Ok(Pass::Replay);
                };
                output.float_samples = result.float_samples;
                output.value = result.value;
                output.annotations = result.annotations;
                output.nonempty = result.nonempty;
            }
            Self::Numeric(step) => {
                let result = step
                    .take()
                    .expect("numeric pass")
                    .finish(account)
                    .map_err(|error| crate::evaluator::numeric_range_error(error, account))?;
                output.float_samples = result.float_samples;
                output.value = result.value.map(OwnedValue::Float);
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
            Self::Mad(step) => {
                let Some(result) = step.finish_pass(control, account)? else {
                    return Ok(Pass::Replay);
                };
                output.float_samples = result.float_samples;
                output.value = result.value.map(OwnedValue::Float);
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
            Self::Quantile(step) => {
                let result = match step.finish_pass(control, account)? {
                    temporal_quantile::Pass::Replay => return Ok(Pass::Replay),
                    temporal_quantile::Pass::Backing(backing) => return Ok(Pass::Backing(backing)),
                    temporal_quantile::Pass::Complete(result) => result,
                };
                output.float_samples = result.float_samples;
                output.value = result.value.map(OwnedValue::Float);
                output.invalid_quantile = result.invalid_quantile;
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
        }
        Ok(Pass::Complete(output))
    }
}

fn fold_error(error: FoldError) -> EvaluationError {
    match error {
        FoldError::Interrupted(error) => error.into(),
        FoldError::Storage(error) => error.into(),
        FoldError::NonAdvancingMerge { kind, .. } => EvaluationError::Invalid(match kind {
            FoldKind::Changes => {
                "changes() cannot advance past equal float and histogram timestamps"
            }
            FoldKind::Resets => "resets() cannot advance past equal float and histogram timestamps",
            _ => unreachable!("typed nonadvancing merge"),
        }),
    }
}

pub(crate) fn scheduled_reader_pass(
    reader: &mut Reader,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl FnMut(SampleRef<'_>) -> Result<(), EvaluationError>,
    drive: impl FnOnce(
        &mut dyn FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, EvaluationError>,
    ) -> Result<(), EvaluationError>,
) -> Result<(), EvaluationError> {
    let mut next = reader.advance(control, account, &mut visit)?;
    drive(&mut |time, value| {
        loop {
            control.check()?;
            account.work(1);
            let derived_matrix::Action::Fetch(fetch) = next else {
                return Ok(true);
            };
            if fetch.time_ms > time {
                return Ok(false);
            }
            if fetch.time_ms == time
                && fetch.occurrence == 0
                && let Some(sample) = value
                && sample.value.kind_bit() == fetch.kind
            {
                reader.receive_value(crate::statistics::ValueParts::new(
                    OwnedValue::from_sample_checked(
                        sample.value,
                        control,
                        account,
                        MemoryKind::Workspace,
                    )?,
                    account,
                ));
            }
            next = reader.advance(control, account, &mut visit)?;
        }
    })?;
    if !matches!(next, derived_matrix::Action::Done) {
        return Err(EvaluationError::Invalid(
            "subquery schedule ended before its matrix reader",
        ));
    }
    Ok(())
}

pub(crate) enum Action<'a> {
    Backing {
        request: temporal_quantile::Backing,
        source: BackingSource,
    },
    ScheduledPass {
        node: NodeId,
        ordinal: usize,
        options: EvaluationOptions,
        grid: Grid,
    },
    Child {
        node: NodeId,
        ordinal: usize,
        options: EvaluationOptions,
    },
    Series {
        sample: Option<EvaluatedSample<'a>>,
        invalid_quantile: Option<f64>,
        annotations: [Option<AnnotationKind>; 6],
        metric: &'a [u8],
        nonempty: bool,
        sample_units: u64,
    },
    Done,
}

#[derive(Clone, Copy)]
pub(crate) struct BackingSource {
    input: Input,
    pub(crate) ordinal: usize,
    pub(crate) kinds: u8,
}
impl BackingSource {
    pub(crate) fn reader(self) -> Reader {
        Reader::new(self.input.grid, self.input.window())
    }
    pub(crate) fn node(self) -> NodeId {
        self.input.body
    }
    pub(crate) fn grid(self) -> Grid {
        self.input.grid
    }
    pub(crate) fn scheduled(self) -> bool {
        self.input.scheduled
    }
    pub(crate) fn options(self, time_ms: i64) -> EvaluationOptions {
        EvaluationOptions {
            time_ms,
            start_ms: self.input.grid.start_ms,
            end_ms: self.input.grid.end_ms,
            step_ms: self.input.grid.interval_ms,
            ..self.input.options
        }
    }
}

pub(crate) struct Step<'a> {
    input: Input,
    _parameter_samples: crate::statistics::OwnedSamples,
    rows: &'a [Descriptor],
    filter: RowFilter<'a>,
    position: usize,
    reader: Reader,
    kernel: Kernel,
    dry: bool,
    scheduled_complete: bool,
    backed: Option<Output>,
    sample_units: u64,
    count_window: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(
        input: Input,
        rows: &'a [Descriptor],
        filter: RowFilter<'a>,
        account: &QueryAccounting,
    ) -> Self {
        let parameter_units = match input.function {
            RangeFunction::Numeric(kind) => kind.parameters() as u64,
            _ => 0,
        };
        Self {
            input,
            _parameter_samples: crate::statistics::OwnedSamples::new(account, parameter_units),
            rows,
            filter,
            position: 0,
            reader: Reader::new(input.grid, input.window()),
            kernel: Kernel::new(input, account),
            dry: true,
            scheduled_complete: false,
            backed: None,
            sample_units: 0,
            count_window: true,
        }
    }

    pub(crate) fn receive_backed(&mut self, value: f64, account: &QueryAccounting) {
        let Kernel::Quantile(step) = &self.kernel else {
            unreachable!("backed quantile continuation")
        };
        let result = step.backed_output(value, account);
        self.backed = Some(Output {
            float_samples: result.float_samples,
            value: result.value.map(OwnedValue::Float),
            invalid_quantile: result.invalid_quantile,
            annotations: [result.annotation, None, None, None, None, None],
            nonempty: result.nonempty,
        });
    }

    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        control.check()?;
        account.work(1);
        Ok(self.reader.receive(sample))
    }

    pub(crate) fn scheduled_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
        drive: impl FnOnce(
            &mut dyn FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, EvaluationError>,
        ) -> Result<(), EvaluationError>,
    ) -> Result<(), EvaluationError> {
        let dry = self.dry;
        let fuse_rank = self.input.scheduled
            && dry
            && matches!(&self.kernel, Kernel::Quantile(step) if step.is_ranking());
        let reader = &mut self.reader;
        let kernel = &mut self.kernel;
        let units = &mut self.sample_units;
        let counting = self.count_window;
        scheduled_reader_pass(
            reader,
            control,
            account,
            |sample| {
                if dry {
                    if fuse_rank {
                        let Kernel::Quantile(step) = kernel else {
                            unreachable!("active quantile rank pass")
                        };
                        step.receive_rank(sample, account);
                    }
                    Ok(())
                } else {
                    if counting {
                        crate::statistics::add_units(
                            units,
                            crate::statistics::weight(sample.value),
                            account,
                        );
                    }
                    kernel.receive(sample, control, account)
                }
            },
            drive,
        )?;
        // Rank counters replace a consuming pass only after the whole child succeeds.
        if fuse_rank {
            self.dry = false;
        }
        self.scheduled_complete = true;
        Ok(())
    }

    fn finish_row(
        &mut self,
        mut output: Output,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        let row = self.rows.get(self.position).expect("active range row");
        let metric = row.labels.get_checked(b"__name__", account, control)?;
        if let RangeFunction::Rate(kind) = self.input.function
            && kind.checks_counter_name()
            && matches!(output.value, Some(OwnedValue::Float(_)))
            && !metric.is_empty()
            && ![
                &b"_total"[..],
                &b"_sum"[..],
                &b"_count"[..],
                &b"_bucket"[..],
            ]
            .iter()
            .any(|suffix| metric.ends_with(suffix))
        {
            *output
                .annotations
                .iter_mut()
                .find(|note| note.is_none())
                .expect("float rate notes") = Some(AnnotationKind::PossibleNonCounter);
        }
        let sample = if let Some(value) = output.value {
            let drop_name = row.drop_name || self.input.drops_name();
            let mut labels = row
                .labels
                .copy_checked(account, MemoryKind::Workspace, control)?;
            if drop_name && !self.input.options.delayed_name_removal {
                drop_reserved(&mut labels, false, account, control)?;
            }
            Some(EvaluatedSample {
                ordinal: row.ordinal,
                labels: labels.into(),
                float_samples: output.float_samples,
                value,
                source_time_ms: self.input.options.time_ms,
                source_start_time_ms: 0,
                drop_name,
            })
        } else {
            None
        };
        self.position += 1;
        self.kernel = Kernel::new(self.input, account);
        self.dry = true;
        let sample_units = std::mem::take(&mut self.sample_units);
        self.count_window = true;
        Ok(Action::Series {
            sample,
            invalid_quantile: output.invalid_quantile.then_some(self.input.parameters[0]),
            annotations: output.annotations,
            metric,
            nonempty: output.nonempty,
            sample_units,
        })
    }

    pub(crate) fn advance(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        loop {
            control.check()?;
            account.work(1);
            let Some(row) = self.rows.get(self.position) else {
                return Ok(Action::Done);
            };
            if !self.filter.accepts(row.ordinal, control, account)? {
                self.position += 1;
                continue;
            }
            if let Some(output) = self.backed.take() {
                return self.finish_row(output, control, account);
            }
            if self.input.scheduled && !self.scheduled_complete {
                return Ok(Action::ScheduledPass {
                    node: self.input.body,
                    ordinal: row.ordinal,
                    grid: self.input.grid,
                    options: EvaluationOptions {
                        time_ms: self.input.grid.end_ms,
                        start_ms: self.input.grid.start_ms,
                        end_ms: self.input.grid.end_ms,
                        step_ms: self.input.grid.interval_ms,
                        ..self.input.options
                    },
                });
            }
            let dry = self.dry;
            let kernel = &mut self.kernel;
            let units = &mut self.sample_units;
            let counting = self.count_window;
            match self.reader.advance(control, account, |sample| {
                if dry {
                    Ok(())
                } else {
                    if counting {
                        crate::statistics::add_units(
                            units,
                            crate::statistics::weight(sample.value),
                            account,
                        );
                    }
                    kernel.receive(sample, control, account)
                }
            })? {
                derived_matrix::Action::Fetch(fetch) => {
                    if fetch.absent_histogram(row.kinds) {
                        continue;
                    }
                    return Ok(Action::Child {
                        node: self.input.body,
                        ordinal: row.ordinal,
                        options: EvaluationOptions {
                            time_ms: fetch.time_ms,
                            start_ms: self.input.grid.start_ms,
                            end_ms: self.input.grid.end_ms,
                            step_ms: self.input.grid.interval_ms,
                            ..self.input.options
                        },
                    });
                }
                derived_matrix::Action::Done => {
                    self.reader = Reader::new(self.input.grid, self.input.window());
                    self.scheduled_complete = false;
                    if self.dry {
                        self.dry = false;
                        continue;
                    }
                    match self.kernel.finish_pass(control, account)? {
                        Pass::Replay => {
                            self.count_window = false;
                            self.dry = true;
                            continue;
                        }
                        Pass::Backing(request) => {
                            return Ok(Action::Backing {
                                request,
                                source: BackingSource {
                                    input: self.input,
                                    ordinal: row.ordinal,
                                    kinds: row.kinds,
                                },
                            });
                        }
                        Pass::Complete(output) => return self.finish_row(output, control, account),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        result::OwnedLabels,
        storage::{StorageError, TimeBounds},
        test_storage::{GeneratedSeries, counter},
    };
    use std::sync::atomic::Ordering;

    type ScheduledVisitor<'a> = dyn for<'sample> FnMut(i64, Option<SampleRef<'sample>>) -> Result<bool, EvaluationError>
        + 'a;

    fn input(count: usize) -> Input {
        let time_ms = count as i64 * 1000;
        Input {
            body: NodeId(0),
            grid: Grid {
                start_ms: 1000,
                end_ms: time_ms,
                interval_ms: 1000,
            },
            options: EvaluationOptions {
                time_ms,
                start_ms: time_ms,
                end_ms: time_ms,
                step_ms: 0,
                lookback_ms: 1,
                start_timestamps: false,
                delayed_name_removal: false,
            },
            function: RangeFunction::Numeric(NumericRangeFunction::Quantile),
            parameters: [0.5, 0.0],
            range_ns: count as i64 * 1_000_000_000,
            offset_ns: 0,
            fixed: false,
            scheduled: true,
        }
    }

    fn row(account: &QueryAccounting) -> Descriptor {
        Descriptor {
            labels: OwnedLabels::new(account, MemoryKind::Workspace),
            ordinal: 0,
            hash: 0,
            drop_name: false,
            first_order: 0,
            sources: 0..1,
            kinds: 1,
            heads: Default::default(),
        }
    }

    #[test]
    fn derived_float_kind_replays_preserve_multiplicity_before_child_bound() {
        let mut calls = [0_u64; 3];
        for (case, kinds) in [1, 0, 3].into_iter().enumerate() {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut rows = [row(&account)];
            rows[0].kinds = kinds;
            let mut input = input(4);
            input.function = RangeFunction::Fold(FoldKind::Sum);
            input.scheduled = false;
            let mut step = Step::new(input, &rows, RowFilter::All, &account);
            let mut outputs = 0;
            loop {
                match step.advance(&control, &account).unwrap() {
                    Action::Child {
                        ordinal, options, ..
                    } => {
                        assert_eq!(ordinal, 0);
                        calls[case] += 1;
                        assert!(calls[case] < 100, "finite derived body requests");
                        if options.time_ms < 4000 {
                            for extra in [0.0, 10.0] {
                                let sample = EvaluatedSample {
                                    ordinal,
                                    labels: OwnedLabels::new(&account, MemoryKind::Workspace)
                                        .into(),
                                    value: OwnedValue::Float(
                                        (options.time_ms / 1000) as f64 + extra,
                                    ),
                                    float_samples: Some(crate::statistics::OwnedSamples::new(
                                        &account, 1,
                                    )),
                                    source_time_ms: options.time_ms,
                                    source_start_time_ms: 0,
                                    drop_name: false,
                                };
                                if step.receive(sample, &control, &account).unwrap() {
                                    break;
                                }
                            }
                        }
                    }
                    Action::Series {
                        sample,
                        invalid_quantile,
                        annotations,
                        metric,
                        nonempty,
                        sample_units,
                    } => {
                        let sample = sample.unwrap();
                        assert_eq!(sample.ordinal, 0);
                        assert!(sample.labels.0.is_empty());
                        assert_eq!(sample.source_time_ms, 4000);
                        assert_eq!(sample.source_start_time_ms, 0);
                        assert!(sample.drop_name);
                        let OwnedValue::Float(value) = sample.value else {
                            panic!("complete derived float sum");
                        };
                        assert_eq!(value.to_bits(), 42.0_f64.to_bits());
                        assert!(
                            invalid_quantile.is_none() && annotations.iter().all(Option::is_none)
                        );
                        assert!(metric.is_empty() && nonempty);
                        assert_eq!(sample_units, 6);
                        outputs += 1;
                    }
                    Action::Done => break,
                    _ => panic!("ordinary unscheduled Float fold"),
                }
            }
            assert_eq!(outputs, 1);
            println!(
                "derived-kind-result mask={kinds} sum=42 units=6 children={}",
                calls[case]
            );
            drop(step);
            drop(rows);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        println!("derived-kind-complete results=3");
        println!(
            "derived-kind-layout reader={} step={} backing_source={} action={}",
            std::mem::size_of::<Reader>(),
            std::mem::size_of::<Step<'_>>(),
            std::mem::size_of::<BackingSource>(),
            std::mem::size_of::<Action<'_>>()
        );
        assert_eq!(&calls[1..], &[28, 28]);
        assert!(
            calls[0] <= 20,
            "Float-only rows must omit empty histogram body replays"
        );
    }

    #[test]
    fn derived_float_kind_keeps_required_requests_and_terminal_cancellation() {
        for kinds in [1, 0, 3, 255] {
            for cancel in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let mut rows = [row(&account)];
                rows[0].kinds = kinds;
                let mut input = input(2);
                input.function = RangeFunction::Fold(FoldKind::Sum);
                input.scheduled = false;
                let mut step = Step::new(input, &rows, RowFilter::All, &account);
                // The third request must still validate the exhausted Float occurrence.
                for occurrence in 0..3 {
                    let Action::Child {
                        ordinal, options, ..
                    } = step.advance(&control, &account).unwrap()
                    else {
                        panic!("required Float acquisition");
                    };
                    assert_eq!((ordinal, options.time_ms), (0, 1000));
                    let mut accepted = false;
                    for value in [1.0, 11.0] {
                        accepted = step
                            .receive(
                                EvaluatedSample {
                                    ordinal,
                                    labels: OwnedLabels::new(&account, MemoryKind::Workspace)
                                        .into(),
                                    value: OwnedValue::Float(value),
                                    float_samples: Some(crate::statistics::OwnedSamples::new(
                                        &account, 1,
                                    )),
                                    source_time_ms: 1000,
                                    source_start_time_ms: 0,
                                    drop_name: false,
                                },
                                &control,
                                &account,
                            )
                            .unwrap();
                        if accepted {
                            break;
                        }
                    }
                    assert_eq!(accepted, occurrence < 2);
                }
                if cancel {
                    control.cancellation.cancel();
                    assert!(matches!(
                        step.advance(&control, &account),
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ));
                } else {
                    let Action::Child { options, .. } = step.advance(&control, &account).unwrap()
                    else {
                        panic!("required next body acquisition");
                    };
                    // Only exact Float-only metadata may omit the histogram fallback.
                    assert_eq!(options.time_ms, if kinds == 1 { 2000 } else { 1000 });
                    if kinds != 1 {
                        let Action::Child { options, .. } =
                            step.advance(&control, &account).unwrap()
                        else {
                            panic!("fresh next-clock Float acquisition");
                        };
                        assert_eq!(options.time_ms, 2000);
                    }
                }
                drop(step);
                drop(rows);
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    fn drive(
        source: &GeneratedSeries,
        control: &Control,
        account: &QueryAccounting,
        visit: &mut ScheduledVisitor<'_>,
    ) -> Result<(), EvaluationError> {
        crate::range_numeric::scan(
            source,
            TimeBounds {
                start_ms: 0,
                end_ms: source.count as i64 * 1000,
            }
            .into(),
            control,
            account,
            |sample| match visit(sample.time_ms, Some(sample)) {
                Ok(_) => Ok(()),
                Err(EvaluationError::Storage(error)) => Err(error),
                Err(EvaluationError::Interrupted(error)) => Err(error.into()),
                Err(error) => panic!("unexpected reader error: {error:?}"),
            },
        )?;
        Ok(())
    }

    fn prime_rank(
        step: &mut Step<'_>,
        source: &GeneratedSeries,
        control: &Control,
        account: &QueryAccounting,
    ) {
        for _ in 0..2 {
            assert!(matches!(
                step.advance(control, account).unwrap(),
                Action::ScheduledPass { .. }
            ));
            step.scheduled_pass(control, account, |visit| {
                drive(source, control, account, visit)
            })
            .unwrap();
        }
        assert!(matches!(
            step.advance(control, account).unwrap(),
            Action::ScheduledPass { .. }
        ));
        assert!(matches!(&step.kernel, Kernel::Quantile(rank) if rank.is_ranking()));
        assert!(step.dry && !step.scheduled_complete && !step.count_window);
        assert_eq!(step.sample_units, source.count as u64);
        assert_eq!(source.counters.opened.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn scheduled_quantile_rank_requires_complete_validation_and_terminal_control() {
        for page in [1, 17, 1024] {
            for failure in ["page", "after", "after_cancel", "terminal"] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let rows = [row(&account)];
                let mut source = GeneratedSeries::new(page + 1, page, counter);
                let mut step = Step::new(input(page + 1), &rows, RowFilter::All, &account);
                prime_rank(&mut step, &source, &control, &account);
                if failure == "page" {
                    source.fail_page = Some(1);
                }
                let result = step.scheduled_pass(&control, &account, |visit| {
                    let result = drive(&source, &control, &account, visit);
                    if failure != "after" {
                        control.cancellation.cancel();
                    }
                    result?;
                    if failure == "terminal" {
                        Ok(())
                    } else {
                        Err(StorageError::Provider(b"after final visitor".to_vec().into()).into())
                    }
                });
                if failure == "terminal" {
                    result.unwrap();
                    assert!(!step.dry && step.scheduled_complete);
                    assert!(matches!(
                        step.advance(&control, &account),
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ));
                } else {
                    assert!(
                        matches!(&result, Err(EvaluationError::Storage(StorageError::Provider(bytes)))
                        if &**bytes == if failure == "page" { &b"injected page failure"[..] } else { &b"after final visitor"[..] })
                    );
                    assert!(step.dry && !step.scheduled_complete);
                    drop(result);
                }
                assert_eq!(step.position, 0);
                assert!(step.backed.is_none());
                assert_eq!(step.sample_units, source.count as u64);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 3);
                assert_eq!(source.counters.closed.load(Ordering::Relaxed), 3);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                if failure != "page" {
                    // A prematurely finished nibble would have cleared these complete counters.
                    assert!(matches!(
                        step.kernel
                            .finish_pass(&Control::default(), &account)
                            .unwrap(),
                        Pass::Replay
                    ));
                }
                drop(step);
                drop(rows);
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn uniform_scheduled_ranks_wait_for_complete_child_success() {
        for page in [1, 17, 1024] {
            for failure in ["page", "after", "after_cancel", "terminal"] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let rows = [row(&account)];
                let mut source =
                    GeneratedSeries::new(page + 1, page, crate::test_storage::two_classes);
                let mut step = Step::new(input(page + 1), &rows, RowFilter::All, &account);
                prime_rank(&mut step, &source, &control, &account);
                step.scheduled_pass(&control, &account, |visit| {
                    drive(&source, &control, &account, visit)
                })
                .unwrap();
                assert!(matches!(
                    step.advance(&control, &account).unwrap(),
                    Action::ScheduledPass { .. }
                ));
                if failure == "page" {
                    source.fail_page = Some(1);
                }
                let result = step.scheduled_pass(&control, &account, |visit| {
                    let result = drive(&source, &control, &account, visit);
                    if failure != "after" {
                        control.cancellation.cancel();
                    }
                    result?;
                    if failure == "terminal" {
                        Ok(())
                    } else {
                        Err(StorageError::Provider(b"after uniform visitor".to_vec().into()).into())
                    }
                });
                if failure == "terminal" {
                    result.unwrap();
                    assert!(!step.dry && step.scheduled_complete);
                    assert!(matches!(
                        step.advance(&control, &account),
                        Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                    ));
                } else {
                    assert!(
                        matches!(&result, Err(EvaluationError::Storage(StorageError::Provider(bytes)))
                        if &**bytes == if failure == "page" { &b"injected page failure"[..] } else { &b"after uniform visitor"[..] })
                    );
                    assert!(step.dry && !step.scheduled_complete);
                    drop(result);
                }
                // The completed keys remain provisional until the entire child succeeds.
                if failure != "page" {
                    let Pass::Complete(output) = step
                        .kernel
                        .finish_pass(&Control::default(), &account)
                        .unwrap()
                    else {
                        panic!("uniform rank pass must still be unconsumed");
                    };
                    let expected: f64 = if (page + 1).is_multiple_of(2) {
                        1.5
                    } else {
                        1.0
                    };
                    assert!(
                        matches!(output.value, Some(OwnedValue::Float(value)) if value.to_bits() == expected.to_bits())
                    );
                }
                assert_eq!(step.position, 0);
                assert!(step.backed.is_none());
                assert_eq!(step.sample_units, (page + 1) as u64);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), 4);
                assert_eq!(source.counters.closed.load(Ordering::Relaxed), 4);
                assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                drop(step);
                drop(rows);
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn scheduled_quantile_rank_uses_one_traversal_per_required_pass() {
        for page in [1, 17, 1024] {
            for (sample, count, q, expected, passes) in [
                (
                    counter as fn(usize) -> crate::value::Sample,
                    2,
                    0.5,
                    1.5f64,
                    4,
                ),
                (crate::test_storage::adjacent_counter, 2, 0.5, 1.0, 4),
                (crate::test_storage::prefix_ladder, 17, 0.0, 0.0, 18),
            ] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let rows = [row(&account)];
                let source = GeneratedSeries::new(count, page, sample);
                let mut input = input(count);
                input.parameters[0] = q;
                let mut step = Step::new(input, &rows, RowFilter::All, &account);
                let mut traversals = 0;
                loop {
                    match step.advance(&control, &account).unwrap() {
                        Action::ScheduledPass { .. } => {
                            traversals += 1;
                            assert!(traversals <= 34);
                            step.scheduled_pass(&control, &account, |visit| {
                                drive(&source, &control, &account, visit)
                            })
                            .unwrap();
                        }
                        Action::Series {
                            sample: Some(sample),
                            sample_units,
                            ..
                        } => {
                            assert_eq!(sample.source_time_ms, count as i64 * 1000);
                            assert!(
                                matches!(sample.value, OwnedValue::Float(value) if value.to_bits() == expected.to_bits())
                            );
                            assert_eq!(sample_units, count as u64);
                            break;
                        }
                        _ => panic!("unexpected quantile continuation"),
                    }
                }
                assert_eq!(traversals, passes);
                assert_eq!(source.counters.opened.load(Ordering::Relaxed), passes);
                assert_eq!(source.counters.closed.load(Ordering::Relaxed), passes);
                assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                drop(step);
                drop(rows);
                account.snapshot().assert_memory_released();
                drop(account);
                engine.snapshot().assert_memory_released();
            }
        }
    }
}
