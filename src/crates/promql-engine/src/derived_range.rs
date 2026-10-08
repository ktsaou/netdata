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
    pub(crate) raw_selector: bool,
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
    value: Option<OwnedValue>,
    invalid_quantile: bool,
    annotations: [Option<AnnotationKind>; 6],
    nonempty: bool,
}
enum Kernel {
    Fold(FoldStep),
    Rate(range_rates::Step),
    Numeric(Option<NumericStep>),
    Mad(mad::Step),
    Quantile(temporal_quantile::Step),
}
impl Kernel {
    fn new(input: Input) -> Self {
        match input.function {
            RangeFunction::Fold(kind) => {
                Self::Fold(FoldStep::new(kind, input.options.start_timestamps))
            }
            RangeFunction::Rate(kind) => {
                Self::Rate(range_rates::Step::new(input.rate_input(), kind))
            }
            RangeFunction::Numeric(NumericRangeFunction::Mad) => Self::Mad(mad::Step::default()),
            RangeFunction::Numeric(NumericRangeFunction::Quantile) => {
                Self::Quantile(temporal_quantile::Step::new(input.parameters[0]))
            }
            RangeFunction::Numeric(kind) => Self::Numeric(Some(NumericStep::new(
                kind,
                input.parameters,
                input.options.time_ms,
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
    ) -> Result<Option<Output>, EvaluationError> {
        let mut output = Output::default();
        match self {
            Self::Fold(step) => {
                let Some(result) = step.finish_pass(control, account).map_err(fold_error)? else {
                    return Ok(None);
                };
                output.value = result.value;
                output.annotations[..2].copy_from_slice(&result.annotations);
                output.nonempty = result.nonempty;
            }
            Self::Rate(step) => {
                let Some(result) = step.finish_pass(control, account)? else {
                    return Ok(None);
                };
                output.value = result.value;
                output.annotations = result.annotations;
                output.nonempty = result.nonempty;
            }
            Self::Numeric(step) => {
                let result = step
                    .take()
                    .expect("numeric pass")
                    .finish()
                    .map_err(|error| crate::evaluator::numeric_range_error(error, account))?;
                output.value = result.value.map(OwnedValue::Float);
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
            Self::Mad(step) => {
                let Some(result) = step.finish_pass(control, account)? else {
                    return Ok(None);
                };
                output.value = result.value.map(OwnedValue::Float);
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
            Self::Quantile(step) => {
                let Some(result) = step
                    .finish_pass(control, account)
                    .map_err(|error| crate::evaluator::numeric_range_error(error, account))?
                else {
                    return Ok(None);
                };
                output.value = result.value.map(OwnedValue::Float);
                output.invalid_quantile = result.invalid_quantile;
                output.annotations[0] = result.annotation;
                output.nonempty = result.nonempty;
            }
        }
        Ok(Some(output))
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

pub(crate) enum Action<'a> {
    RawPass {
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
        sample: Option<EvaluatedSample>,
        invalid_quantile: Option<f64>,
        annotations: [Option<AnnotationKind>; 6],
        metric: &'a [u8],
        nonempty: bool,
    },
    Done,
}

pub(crate) struct Step<'a> {
    input: Input,
    rows: &'a [Descriptor],
    filter: RowFilter<'a>,
    position: usize,
    reader: Reader,
    kernel: Kernel,
    dry: bool,
    raw_complete: bool,
}
impl<'a> Step<'a> {
    pub(crate) fn new(input: Input, rows: &'a [Descriptor], filter: RowFilter<'a>) -> Self {
        Self {
            input,
            rows,
            filter,
            position: 0,
            reader: Reader::new(input.grid, input.window()),
            kernel: Kernel::new(input),
            dry: true,
            raw_complete: false,
        }
    }

    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        control.check()?;
        account.work(1);
        Ok(self.reader.receive(sample))
    }

    pub(crate) fn raw_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
        drive: impl FnOnce(
            &mut dyn FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, EvaluationError>,
        ) -> Result<(), EvaluationError>,
    ) -> Result<(), EvaluationError> {
        let dry = self.dry;
        let reader = &mut self.reader;
        let kernel = &mut self.kernel;
        let mut next = reader.advance(control, account, |sample| {
            if dry {
                Ok(())
            } else {
                kernel.receive(sample, control, account)
            }
        })?;
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
                    reader.receive_value(OwnedValue::from_sample_checked(
                        sample.value,
                        control,
                        account,
                        MemoryKind::Workspace,
                    )?);
                }
                next = reader.advance(control, account, |sample| {
                    if dry {
                        Ok(())
                    } else {
                        kernel.receive(sample, control, account)
                    }
                })?;
            }
        })?;
        if !matches!(next, derived_matrix::Action::Done) {
            return Err(EvaluationError::Invalid(
                "raw subquery schedule ended before its matrix reader",
            ));
        }
        self.raw_complete = true;
        Ok(())
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
            if !self.filter.accepts(row.ordinal) {
                self.position += 1;
                continue;
            }
            if self.input.raw_selector && !self.raw_complete {
                return Ok(Action::RawPass {
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
            match self.reader.advance(control, account, |sample| {
                if dry {
                    Ok(())
                } else {
                    kernel.receive(sample, control, account)
                }
            })? {
                derived_matrix::Action::Fetch(fetch) => {
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
                    self.raw_complete = false;
                    if self.dry {
                        self.dry = false;
                        continue;
                    }
                    let Some(mut output) = self.kernel.finish_pass(control, account)? else {
                        self.dry = true;
                        continue;
                    };
                    let metric = row
                        .labels
                        .0
                        .iter()
                        .find(|label| label.name.as_ref() == b"__name__")
                        .map_or(b"".as_slice(), |label| label.value.as_ref());
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
                        let mut labels =
                            row.labels
                                .copy_checked(account, MemoryKind::Workspace, control)?;
                        if drop_name && !self.input.options.delayed_name_removal {
                            drop_reserved(&mut labels, false, account, control)?;
                        }
                        Some(EvaluatedSample {
                            ordinal: row.ordinal,
                            labels,
                            value,
                            source_time_ms: self.input.options.time_ms,
                            source_start_time_ms: 0,
                            drop_name,
                        })
                    } else {
                        None
                    };
                    self.position += 1;
                    self.kernel = Kernel::new(self.input);
                    self.dry = true;
                    return Ok(Action::Series {
                        sample,
                        invalid_quantile: output
                            .invalid_quantile
                            .then_some(self.input.parameters[0]),
                        annotations: output.annotations,
                        metric,
                        nonempty: output.nonempty,
                    });
                }
            }
        }
    }
}
