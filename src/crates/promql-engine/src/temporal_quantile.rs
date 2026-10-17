// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned promql/functions.go:1652-1673 and quantile.go:717-738; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    annotations::AnnotationKind,
    numerical::quiet_nan,
    range_numeric::{NumericRangeError, NumericRangeInput, NumericRangeOutput, scan},
    range_quantile_rank::{NAN_KEY, Plan, Selector, ZERO_KEY, ordered_key, ordered_value},
    storage::{Control, QueryProvider, Series, StorageError},
    temporal_quantile_sort::all_nan_ordinal,
    value::{SampleRef, SampleValueRef},
};

const SOURCE_NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

#[derive(Default)]
struct Membership {
    count: u64,
    nan_count: u64,
    below_zero: u64,
    histograms: bool,
    first_key: Option<u64>,
    one_class: bool,
    first_nan: Option<u64>,
    one_nan: bool,
    zero_signs: u8,
    first_nan_samples: Option<crate::statistics::OwnedSamples>,
}
impl Membership {
    fn receive(&mut self, value: f64, account: &QueryAccounting) -> Result<(), StorageError> {
        let count = self
            .count
            .checked_add(1)
            .ok_or(StorageError::Invalid("quantile sample count overflow"))?;
        let key = ordered_key(value);
        if let Some(first) = self.first_key {
            self.one_class &= first == key;
        } else {
            self.first_key = Some(key);
            self.one_class = true;
        }
        if value.is_nan() {
            self.nan_count += 1;
            let quiet = quiet_nan(value).to_bits();
            if let Some(first) = self.first_nan {
                self.one_nan &= first == quiet;
            } else {
                self.first_nan = Some(quiet);
                self.first_nan_samples = Some(crate::statistics::OwnedSamples::new(account, 1));
                self.one_nan = true;
            }
        } else if value == 0.0 {
            self.zero_signs |= if value.is_sign_negative() { 2 } else { 1 };
        }
        if value.is_nan() || value < 0.0 {
            self.below_zero += 1;
        }
        self.count = count;
        Ok(())
    }

    fn value(&self, key: u64) -> Option<f64> {
        match key {
            NAN_KEY => self
                .one_nan
                .then(|| f64::from_bits(self.first_nan.expect("NaN class"))),
            ZERO_KEY => match self.zero_signs {
                1 => Some(0.0),
                2 => Some(-0.0),
                _ => None,
            },
            _ => Some(ordered_value(key)),
        }
    }
}

#[derive(Clone, Copy)]
struct Selection {
    key: Option<u64>,
    ordinal: u64,
}

enum Phase {
    Membership,
    Rank(Selector),
    Fetch {
        selections: [Option<Selection>; 2],
        next: u64,
        seen: [u64; 2],
        values: [Option<f64>; 2],
        samples: [Option<crate::statistics::OwnedSamples>; 2],
    },
}

pub(crate) struct Step {
    quantile: f64,
    _quantile_samples: crate::statistics::OwnedSamples,
    membership: Membership,
    plan: Option<Plan>,
    phase: Phase,
}
#[derive(Clone, Copy)]
pub(crate) struct Backing {
    pub(crate) count: u64,
    pub(crate) plan: Plan,
}
pub(crate) enum Pass {
    Replay,
    Complete(NumericRangeOutput),
    Backing(Backing),
}
impl Step {
    pub(crate) fn new(quantile: f64, account: &QueryAccounting) -> Self {
        Self {
            quantile,
            _quantile_samples: crate::statistics::OwnedSamples::new(account, 1),
            membership: Membership::default(),
            plan: None,
            phase: Phase::Membership,
        }
    }

    pub(crate) fn is_ranking(&self) -> bool {
        matches!(self.phase, Phase::Rank(_))
    }

    pub(crate) fn receive_rank(&mut self, sample: SampleRef<'_>, account: &QueryAccounting) {
        let Phase::Rank(selector) = &mut self.phase else {
            unreachable!("active quantile rank pass")
        };
        if sample.value.is_stale() {
            return;
        }
        account.work(1);
        if let SampleValueRef::Float(value) = sample.value {
            selector.receive(value, account);
        }
    }

    pub(crate) fn receive(
        &mut self,
        sample: SampleRef<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        control.check()?;
        if sample.value.is_stale() {
            return Ok(());
        }
        account.work(1);
        if let SampleValueRef::Float(value) = sample.value {
            match &mut self.phase {
                Phase::Membership => self.membership.receive(value, account)?,
                Phase::Rank(selector) => selector.receive(value, account),
                Phase::Fetch {
                    selections,
                    next,
                    seen,
                    values,
                    samples,
                } => {
                    for lane in 0..2 {
                        if let Some(selection) = selections[lane]
                            && selection.key.is_none_or(|key| key == ordered_key(value))
                        {
                            if seen[lane] == selection.ordinal {
                                values[lane] = Some(value);
                                samples[lane].get_or_insert_with(|| {
                                    crate::statistics::OwnedSamples::new(account, 1)
                                });
                            }
                            seen[lane] += 1;
                        }
                    }
                    *next = next
                        .checked_add(1)
                        .ok_or(StorageError::Invalid("quantile replay count overflow"))?;
                }
            }
        } else if matches!(self.phase, Phase::Membership) {
            self.membership.histograms = true;
        }
        Ok(())
    }

    fn output(&self, value: Option<f64>, account: &QueryAccounting) -> NumericRangeOutput {
        NumericRangeOutput {
            float_samples: value.map(|_| crate::statistics::OwnedSamples::new(account, 1)),
            progress: None,
            value,
            invalid_quantile: self.membership.count > 0 && !(0.0..=1.0).contains(&self.quantile),
            annotation: (self.membership.histograms && self.membership.count > 0)
                .then_some(AnnotationKind::HistogramIgnoredInMixedRange),
            nonempty: self.membership.count > 0 || self.membership.histograms,
        }
    }

    pub(crate) fn finish_pass(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Pass, StorageError> {
        control.check().map_err(StorageError::from)?;
        match &mut self.phase {
            Phase::Membership => {
                if self.membership.count == 0 {
                    return Ok(Pass::Complete(self.output(None, account)));
                }
                if !(0.0..=1.0).contains(&self.quantile) {
                    let value = if self.quantile < 0.0 {
                        f64::NEG_INFINITY
                    } else if self.quantile > 1.0 {
                        f64::INFINITY
                    } else {
                        SOURCE_NAN
                    };
                    return Ok(Pass::Complete(self.output(Some(value), account)));
                }
                let plan = Plan::new(self.quantile, self.membership.count);
                self.plan = Some(plan);
                let key = self.membership.first_key.expect("nonempty float window");
                if self.membership.one_class {
                    if let Some(value) = self.membership.value(key) {
                        return Ok(Pass::Complete(
                            self.output(Some(plan.interpolate([value; 2])), account),
                        ));
                    }
                    let mut ordinals = plan.ordinals;
                    if key == NAN_KEY {
                        for ordinal in &mut ordinals {
                            *ordinal =
                                all_nan_ordinal(self.membership.count, *ordinal, control, account)?;
                        }
                    }
                    self.phase = Phase::Fetch {
                        selections: ordinals.map(|ordinal| Some(Selection { key: None, ordinal })),
                        next: 0,
                        seen: [0; 2],
                        values: [None; 2],
                        samples: [None, None],
                    };
                } else {
                    self.phase = Phase::Rank(Selector::new(plan));
                }
                Ok(Pass::Replay)
            }
            Phase::Rank(selector) => {
                let Some(keys) = selector.finish_pass(control, account)? else {
                    return Ok(Pass::Replay);
                };
                let values = keys.map(|key| self.membership.value(key));
                let plan = self.plan.expect("quantile rank plan");
                let [Some(lower), Some(upper)] = values else {
                    if self.membership.count > 12 {
                        return Ok(Pass::Backing(Backing {
                            count: self.membership.count,
                            plan,
                        }));
                    }
                    let selections = std::array::from_fn(|lane| {
                        if values[lane].is_some() {
                            return None;
                        }
                        let ordinal = if keys[lane] == NAN_KEY {
                            self.membership.nan_count - 1 - plan.ordinals[lane]
                        } else {
                            plan.ordinals[lane] - self.membership.below_zero
                        };
                        Some(Selection {
                            key: Some(keys[lane]),
                            ordinal,
                        })
                    });
                    self.phase = Phase::Fetch {
                        selections,
                        next: 0,
                        seen: [0; 2],
                        values,
                        samples: values.map(|value| {
                            value.map(|_| crate::statistics::OwnedSamples::new(account, 1))
                        }),
                    };
                    return Ok(Pass::Replay);
                };
                Ok(Pass::Complete(
                    self.output(Some(plan.interpolate([lower, upper])), account),
                ))
            }
            Phase::Fetch { next, values, .. } => {
                if *next != self.membership.count {
                    return Err(StorageError::Invalid("quantile replay membership changed"));
                }
                let [Some(lower), Some(upper)] = *values else {
                    return Err(StorageError::Invalid("quantile selected ordinal missing"));
                };
                let plan = self.plan.expect("quantile ordinal plan");
                Ok(Pass::Complete(
                    self.output(Some(plan.interpolate([lower, upper])), account),
                ))
            }
        }
    }
    pub(crate) fn backed_output(
        &self,
        value: f64,
        account: &QueryAccounting,
    ) -> NumericRangeOutput {
        self.output(Some(value), account)
    }
}

pub(crate) fn evaluate<S: Series, Q: QueryProvider>(
    query: &Q,
    series: &S,
    input: NumericRangeInput,
    quantile: f64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<NumericRangeOutput, NumericRangeError> {
    let mut input = input;
    let mut step = Step::new(quantile, account);
    loop {
        let progress = scan(series, input, control, account, |sample| {
            step.receive(sample, control, account)
        })?;
        match step.finish_pass(control, account)? {
            Pass::Complete(mut output) => {
                output.progress = progress;
                return Ok(output);
            }
            Pass::Backing(backing) => {
                input.matrix = input.matrix.map(|window| window.with_progress(progress));
                let mut final_progress = None;
                let value = crate::temporary_float_bits::quantile(
                    query,
                    backing.count,
                    backing.plan,
                    control,
                    account,
                    |writer| {
                        final_progress = scan(series, input, control, account, |sample| {
                            if let SampleValueRef::Float(value) = sample.value
                                && !sample.value.is_stale()
                            {
                                writer.push(value.to_bits())?;
                            }
                            Ok(())
                        })?;
                        Ok::<_, NumericRangeError>(())
                    },
                )?;
                let mut output = step.backed_output(value, account);
                output.progress = final_progress;
                return Ok(output);
            }
            Pass::Replay => (),
        }
        input.matrix = input.matrix.map(|window| window.with_progress(progress));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::FloatHistogram,
        storage::{Interrupted, TimeBounds},
        test_storage::{GeneratedSeries, counter},
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };
    use std::sync::atomic::Ordering;

    fn window(count: usize) -> NumericRangeInput {
        TimeBounds {
            start_ms: 0,
            end_ms: count as i64 * 1000,
        }
        .into()
    }
    fn nan(index: usize) -> Sample {
        Sample {
            value: SampleValue::Float(f64::from_bits(0x7ff8_0000_0000_0001 + index as u64)),
            ..counter(index)
        }
    }
    fn histogram(index: usize) -> Sample {
        Sample {
            value: SampleValue::FloatHistogram(FloatHistogram::default()),
            ..counter(index)
        }
    }
    fn mixed(index: usize) -> Sample {
        if index == 0 {
            counter(index)
        } else {
            histogram(index)
        }
    }
    fn stale(index: usize) -> Sample {
        Sample {
            value: SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
            ..counter(index)
        }
    }
    fn zero(index: usize) -> Sample {
        Sample {
            value: SampleValue::Float(if index.is_multiple_of(2) { -0.0 } else { 0.0 }),
            ..counter(index)
        }
    }

    fn short_zero(index: usize) -> Sample {
        const BITS: [u64; 4] = [
            0x8000_0000_0000_0001,
            0x8000_0000_0000_0000,
            0x8000_0000_0000_0000,
            0,
        ];
        Sample {
            value: SampleValue::Float(f64::from_bits(BITS[index])),
            ..counter(index)
        }
    }

    fn short_mixed(index: usize) -> Sample {
        const BITS: [u64; 12] = [
            0xfff0_0000_0000_0045,
            0x7ff0_0000_0000_0044,
            0x3ff0_0000_0000_0000,
            0xfff0_0000_0000_0045,
            0xc008_0000_0000_0000,
            0xc008_0000_0000_0000,
            0x8000_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0x3ff0_0000_0000_0000,
            0xfff8_0000_0000_0043,
            0xc008_0000_0000_0000,
            0,
        ];
        Sample {
            value: SampleValue::Float(f64::from_bits(BITS[index])),
            ..counter(index)
        }
    }

    fn mixed_zeros(index: usize) -> Sample {
        let value = [-1.0, -0.0, 0.0, 1.0][index % 4];
        Sample {
            value: SampleValue::Float(value),
            ..counter(index)
        }
    }

    #[test]
    fn provisional_rank_accounts_samples_without_adding_a_control_boundary() {
        use crate::value::SampleValueRef;

        fn ranked(account: &QueryAccounting) -> Step {
            let mut step = Step::new(0.5, account);
            assert!(!step.is_ranking());
            for index in 0..2 {
                step.receive(counter(index).as_ref(), &Control::default(), account)
                    .unwrap();
            }
            assert!(matches!(
                step.finish_pass(&Control::default(), account).unwrap(),
                Pass::Replay
            ));
            assert!(step.is_ranking());
            step
        }

        let histogram = FloatHistogram::default();
        for (value, work) in [
            (SampleValueRef::Float(1.0), 4),
            (SampleValueRef::FloatHistogram(histogram.as_ref()), 1),
            (SampleValueRef::Float(f64::from_bits(STALE_NAN_BITS)), 0),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut step = ranked(&account);
            let before = account.snapshot();
            step.receive_rank(
                SampleRef {
                    time_ms: 1000,
                    start_time_ms: 0,
                    value,
                },
                &account,
            );
            let after = account.snapshot();
            assert_eq!(after.work - before.work, work);
            assert_eq!(after.memory, before.memory);
            assert_eq!(
                after.statistics.samples.total_queryable_samples,
                before.statistics.samples.total_queryable_samples
            );
            drop(step);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
        for budget in 1..=4 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut step = ranked(&account);
            let control = Control::default();
            let before = account.snapshot().work;
            crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                step.receive_rank(counter(0).as_ref(), &account);
            });
            assert_eq!(account.snapshot().work - before, 4);
            assert!(matches!(control.check(), Err(Interrupted::Cancelled)));
            assert!(matches!(
                step.finish_pass(&control, &account),
                Err(StorageError::Interrupted(Interrupted::Cancelled))
            ));
            drop(step);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn generated_ambiguous_history_uses_flat_ram_and_provider_owned_extent() {
        use crate::{
            execution::{Engine, QueryRequest},
            test_storage::FixtureProvider,
        };
        fn seek_index(time_ms: i64) -> usize {
            usize::try_from(time_ms.saturating_sub(1).max(0) / 1000).unwrap()
        }
        for computed in [false, true] {
            for page in [1, 17, 1024] {
                let mut previous = None;
                for count in [2048, 100_000] {
                    let engine = Engine::default();
                    let expression = if computed {
                        format!("quantile_over_time(0.5,(counter_total*1)[{count}s:1s])")
                    } else {
                        "quantile_over_time(0.5,counter_total[100000s])".to_owned()
                    };
                    let query = engine
                        .compile(
                            expression.as_bytes(),
                            crate::execution::EngineOptions {
                                lookback_ms: 1,
                                ..crate::execution::EngineOptions::default()
                            },
                            &Control::default(),
                        )
                        .unwrap();
                    let mut provider = FixtureProvider::new(count, page, 1);
                    provider.series[0].source.sample = mixed_zeros;
                    provider.series[0].source.seek_index = Some(seek_index);
                    let report = query.collect_reported(
                        &provider,
                        QueryRequest::Instant {
                            time_ms: count as i64 * 1000,
                        },
                        &Control::default(),
                    );
                    let output = report.result.as_ref().unwrap();
                    assert_eq!(output.series.len(), 1);
                    assert_eq!(output.series[0].labels.0.len(), 2);
                    assert_eq!(&*output.series[0].labels.0[0].name, b"instance");
                    assert_eq!(&*output.series[0].labels.0[0].value, b"0");
                    assert_eq!(&*output.series[0].labels.0[1].name, b"job");
                    assert_eq!(&*output.series[0].labels.0[1].value, b"worker");
                    assert_eq!(output.series[0].points.len(), 1);
                    assert_eq!(output.series[0].points[0].time_ms, count as i64 * 1000);
                    let crate::result::OwnedValue::Float(value) = output.series[0].points[0].value
                    else {
                        panic!("float quantile");
                    };
                    assert_eq!(value.to_bits(), 0);
                    assert!(report.annotations.is_empty());
                    let snapshot = &report.observations;
                    assert_eq!(snapshot.output_samples, 1);
                    assert_eq!(
                        snapshot.statistics.samples.total_queryable_samples,
                        count as u64
                    );
                    assert!(snapshot.statistics.samples.samples_read <= snapshot.samples_fetched);
                    let peaks = [
                        MemoryKind::Workspace,
                        MemoryKind::ProviderHeld,
                        MemoryKind::BorrowedView,
                    ]
                    .map(|kind| snapshot.memory[kind as usize].peak);
                    if let Some(previous) = previous {
                        assert_eq!(peaks, previous);
                    }
                    previous = Some(peaks);
                    assert_eq!(snapshot.temporary_storage.extent.peak, count as u64 * 8);
                    assert_eq!(snapshot.temporary_storage.extent.current, 0);
                    assert_eq!(snapshot.temporary_storage.stores.peak, 1);
                    assert_eq!(snapshot.temporary_storage.stores.current, 0);
                    assert!(snapshot.temporary_storage.read_calls > 0);
                    assert!(snapshot.temporary_storage.written_bytes >= count as u64 * 8);
                    let counters = &provider.series[0].source.counters;
                    if computed {
                        assert_eq!(
                            snapshot.samples_fetched,
                            counters.samples_returned.load(Ordering::Relaxed) as u64
                        );
                        for kind in [
                            MemoryKind::Workspace,
                            MemoryKind::ProviderHeld,
                            MemoryKind::BorrowedView,
                        ] {
                            assert_eq!(snapshot.memory[kind as usize].current, 0);
                        }
                    }
                    assert_eq!(counters.active.load(Ordering::Relaxed), 0);
                    assert_eq!(counters.peak.load(Ordering::Relaxed), 1);
                    let measured = (
                        snapshot.samples_fetched,
                        snapshot.pages,
                        counters.opened.load(Ordering::Relaxed),
                        snapshot.work,
                        snapshot.memory[MemoryKind::Workspace as usize].peak,
                    );
                    provider.assert_closed();
                    drop(report);
                    drop(query);
                    engine.accounting.snapshot().assert_memory_released();
                    println!(
                        "{{\"derived_quantile_points\":{count},\"computed\":{computed},\"page_size\":{page},\"fetched\":{},\"pages\":{},\"cursors\":{},\"work\":{},\"workspace_peak\":{}}}",
                        measured.0, measured.1, measured.2, measured.3, measured.4
                    );
                    if computed {
                        assert!(
                            measured.0 <= 50 * count as u64,
                            "scheduled quantile ranks must avoid a second traversal: {measured:?} for {count} points/page{page}"
                        );
                        assert!(measured.1 <= 50 * count.div_ceil(page) as u64);
                        assert!(measured.2 <= 50);
                    }
                }
            }
        }
    }

    #[test]
    fn short_mixed_origin_controls_match_pinned_source() {
        type SourceCase = (fn(usize) -> Sample, usize, [u64; 3]);
        let cases: [SourceCase; 2] = [
            (
                short_zero,
                4,
                [0x8000_0000_0000_0001, 0x8000_0000_0000_0000, 0],
            ),
            (
                short_mixed,
                12,
                [
                    0xfff8_0000_0000_0045,
                    0xc008_0000_0000_0000,
                    0x3ff0_0000_0000_0000,
                ],
            ),
        ];
        for page in [1, 17, 1024] {
            for (sample, count, expected) in cases {
                for (q, bits) in [0.0, 0.5, 1.0].into_iter().zip(expected) {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let source = GeneratedSeries::new(count, page, sample);
                    let output = evaluate(
                        &crate::test_storage::NoTemporary,
                        &source,
                        window(count),
                        q,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    assert_eq!(output.value.unwrap().to_bits(), bits, "q{q}/page{page}");
                    assert!(!output.invalid_quantile);
                    assert_eq!(output.annotation, None);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    account.snapshot().assert_memory_released();
                    drop(output);
                    assert_eq!(account.current_samples(), 0);
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn long_rank_and_origin_replays_keep_one_page_and_constant_memory() {
        type SourceCase = (fn(usize) -> Sample, f64, [u64; 2], [u64; 2]);
        let cases: [SourceCase; 5] = [
            (
                counter,
                0.5,
                [1024.5f64.to_bits(), 50_000.5f64.to_bits()],
                [8, 9],
            ),
            // Adjacent low-bit keys retain the complete sixteen-pass rank path.
            (
                crate::test_storage::adjacent_counter,
                0.5,
                [1.0f64.to_bits(); 2],
                [17; 2],
            ),
            (
                crate::test_storage::two_classes,
                0.5,
                [1.5f64.to_bits(); 2],
                [3; 2],
            ),
            // The terminal source sort origins are separately pinned by the inverse-sort controls.
            (
                nan,
                1.0,
                [0x7ff8_0000_0000_03fc, 0x7ff8_0000_0000_c34c],
                [2; 2],
            ),
            (zero, 1.0, [0, 0], [2; 2]),
        ];
        for page in [1, 17, 1024] {
            for (sample, q, expected, passes) in cases {
                let mut previous = None;
                for ((count, bits), passes) in [2048, 100_000].into_iter().zip(expected).zip(passes)
                {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let source = GeneratedSeries::new(count, page, sample);
                    let result = evaluate(
                        &crate::test_storage::NoTemporary,
                        &source,
                        window(count),
                        q,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    assert_eq!(
                        result.value.unwrap().to_bits(),
                        bits,
                        "count{count}/page{page}"
                    );
                    assert!(!result.invalid_quantile);
                    assert_eq!(result.annotation, None);
                    let snapshot = account.snapshot();
                    let peaks = [
                        MemoryKind::Workspace,
                        MemoryKind::ProviderHeld,
                        MemoryKind::BorrowedView,
                    ]
                    .map(|kind| snapshot.memory[kind as usize].peak);
                    if let Some(previous) = previous {
                        assert_eq!(peaks, previous);
                    }
                    previous = Some(peaks);
                    assert_eq!(snapshot.samples_fetched, passes * count as u64);
                    assert_eq!(
                        source.counters.opened.load(Ordering::Relaxed),
                        passes as usize
                    );
                    assert_eq!(
                        source.counters.closed.load(Ordering::Relaxed),
                        passes as usize
                    );
                    assert_eq!(source.counters.peak.load(Ordering::Relaxed), 1);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    snapshot.assert_memory_released();
                    drop(result);
                    assert_eq!(account.current_samples(), 0);
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn membership_and_both_notes_precede_quantile_shortcuts() {
        for page in [1, 17, 1024] {
            for (sample, has_floats, has_mixed) in [
                (counter as fn(usize) -> Sample, true, false),
                (histogram, false, false),
                (mixed, true, true),
                (stale, false, false),
            ] {
                for q in [-1.0, 2.0, SOURCE_NAN] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let source = GeneratedSeries::new(3, page, sample);
                    let output = evaluate(
                        &crate::test_storage::NoTemporary,
                        &source,
                        window(3),
                        q,
                        &Control::default(),
                        &account,
                    )
                    .unwrap();
                    assert_eq!(output.invalid_quantile, has_floats);
                    assert_eq!(output.value.is_some(), has_floats);
                    assert_eq!(
                        output.annotation,
                        has_mixed.then_some(AnnotationKind::HistogramIgnoredInMixedRange)
                    );
                    assert_eq!(source.counters.opened.load(Ordering::Relaxed), 1);
                    assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
                    account.snapshot().assert_memory_released();
                    drop(output);
                    assert_eq!(account.current_samples(), 0);
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn invalid_quantile_does_not_hide_late_provider_failure_or_cancellation() {
        for page in [1, 17, 1024] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let mut source = GeneratedSeries::new(page + 1, page, nan);
            source.fail_page = Some(1);
            let result = evaluate(
                &crate::test_storage::NoTemporary,
                &source,
                window(page + 1),
                -1.0,
                &Control::default(),
                &account,
            );
            assert!(matches!(
                result,
                Err(NumericRangeError::Storage(StorageError::Provider(_)))
            ));
            drop(result);
            let control = Control::default();
            control.cancellation.cancel();
            let result = evaluate(
                &crate::test_storage::NoTemporary,
                &source,
                window(page + 1),
                SOURCE_NAN,
                &control,
                &account,
            );
            assert!(matches!(
                result,
                Err(NumericRangeError::Storage(StorageError::Interrupted(
                    Interrupted::Cancelled
                )))
            ));
            assert_eq!(source.counters.active.load(Ordering::Relaxed), 0);
            assert_eq!(
                source.counters.opened.load(Ordering::Relaxed),
                source.counters.closed.load(Ordering::Relaxed)
            );
            drop(result);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }
}
