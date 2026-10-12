// SPDX-License-Identifier: GPL-3.0-or-later
//! Execution phases and float-equivalent sample units are independent of fetched pages and RAM bytes.
use crate::{accounting::QueryAccounting, histogram::CUSTOM_BUCKETS_SCHEMA, value::SampleValueRef};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Timings {
    pub eval_total_time: Duration,
    pub result_sort_time: Duration,
    pub query_preparation_time: Duration,
    pub inner_eval_time: Duration,
    pub exec_queue_time: Duration,
    pub exec_total_time: Duration,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Samples {
    pub total_queryable_samples: u64,
    pub samples_read: u64,
    pub peak_samples: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Statistics {
    pub timings: Timings,
    pub samples: Samples,
}

pub(crate) struct PrimaryProgress<'a> {
    account: &'a QueryAccounting,
    base: u64,
    publish: bool,
    units: std::cell::Cell<u64>,
}
impl<'a> PrimaryProgress<'a> {
    pub(crate) fn new(account: &'a QueryAccounting, base: u64, publish: bool) -> Self {
        Self {
            account,
            base,
            publish,
            units: std::cell::Cell::new(0),
        }
    }
    pub(crate) fn add(&self, units: u64) {
        let mut next = self.units.get();
        add_units(&mut next, units, self.account);
        self.units.set(next);
        if self.publish {
            let mut total = self.base;
            add_units(&mut total, next, self.account);
            self.account.queryable_samples(total);
        }
    }
    pub(crate) fn units(&self) -> u64 {
        self.units.get()
    }
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum Phase {
    Evaluation,
    Sort,
    Preparation,
    Inner,
    Execution,
}
pub(crate) const PHASES: usize = 5;
impl Timings {
    pub(crate) fn add(&mut self, phase: Phase, elapsed: Duration) {
        let field = match phase {
            Phase::Evaluation => &mut self.eval_total_time,
            Phase::Sort => &mut self.result_sort_time,
            Phase::Preparation => &mut self.query_preparation_time,
            Phase::Inner => &mut self.inner_eval_time,
            Phase::Execution => &mut self.exec_total_time,
        };
        *field = field.saturating_add(elapsed);
    }
}

fn len<T>(slice: Option<&[T]>) -> u64 {
    slice.map_or(0, |slice| slice.len() as u64)
}
fn histogram_weight(lengths: [u64; 5]) -> u64 {
    // Pinned Go AMD64 FloatHistogram.Size(), with the HPoint timestamp and two float fields per unit.
    lengths.into_iter().fold(176_u64, |bytes, length| {
        bytes.saturating_add(length.saturating_mul(8))
    }) / 16
}
pub(crate) fn weight(value: SampleValueRef<'_>) -> u64 {
    match value {
        SampleValueRef::Float(_) => 1,
        SampleValueRef::FloatHistogram(value) => histogram_weight([
            len(value.positive_spans),
            len(value.negative_spans),
            len(value.positive_buckets),
            len(value.negative_buckets),
            len(value.custom_values),
        ]),
        SampleValueRef::IntegerHistogram(value) => {
            let custom = value.schema == CUSTOM_BUCKETS_SCHEMA;
            histogram_weight([
                len(value.positive_spans),
                if custom { 0 } else { len(value.negative_spans) },
                len(value.positive_buckets),
                if custom {
                    0
                } else {
                    len(value.negative_buckets)
                },
                if custom { len(value.custom_values) } else { 0 },
            ])
        }
    }
}
pub(crate) fn add_units(total: &mut u64, units: u64, account: &QueryAccounting) {
    if let Some(next) = total.checked_add(units) {
        *total = next;
    } else {
        *total = u64::MAX;
        account.statistics_overflow();
    }
}

#[derive(Debug)]
pub(crate) struct OwnedSamples {
    account: QueryAccounting,
    units: u64,
}
impl OwnedSamples {
    pub(crate) fn new(account: &QueryAccounting, units: u64) -> Self {
        let mut lease = Self {
            account: account.clone(),
            units: 0,
        };
        lease.resize(units);
        lease
    }
    pub(crate) fn resize(&mut self, units: u64) {
        self.account.resize_samples(self.units, units);
        self.units = units;
    }
    pub(crate) fn add(&mut self, units: u64) {
        let mut next = self.units;
        add_units(&mut next, units, &self.account);
        self.resize(next);
    }
    pub(crate) fn units(&self) -> u64 {
        self.units
    }
    pub(crate) fn copy(&self) -> Self {
        Self::new(self.accounting(), self.units)
    }
    pub(crate) fn accounting(&self) -> &QueryAccounting {
        &self.account
    }
    pub(crate) fn absorb(&mut self, mut source: Self) {
        let units = source.units();
        source.resize(0);
        self.add(units);
    }
}
impl Drop for OwnedSamples {
    fn drop(&mut self) {
        self.resize(0);
    }
}

pub(crate) fn hold_float(
    account: &QueryAccounting,
    value: SampleValueRef<'_>,
) -> Option<OwnedSamples> {
    matches!(value, SampleValueRef::Float(_)).then(|| OwnedSamples::new(account, 1))
}

#[derive(Debug)]
pub(crate) struct ValueParts {
    pub(crate) value: crate::result::OwnedValue,
    pub(crate) float_samples: Option<OwnedSamples>,
}
impl std::ops::Deref for ValueParts {
    type Target = crate::result::OwnedValue;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
impl ValueParts {
    pub(crate) fn new(value: crate::result::OwnedValue, account: &QueryAccounting) -> Self {
        Self {
            float_samples: hold_float(account, value.as_ref()),
            value,
        }
    }
}

#[derive(Default)]
pub(crate) struct FloatParameters {
    pub(crate) values: [f64; 2],
    pub(crate) samples: [Option<OwnedSamples>; 2],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogram, IntegerHistogram, Span},
    };

    #[test]
    fn weights_use_admitted_slice_lengths_and_conversion_fields() {
        let spans = [Span {
            offset: 0,
            length: 4,
        }];
        let buckets = [1.0, 2.0, 3.0, 4.0];
        let source = FloatHistogram::default();
        let mut value = source.as_ref();
        assert_eq!(weight(SampleValueRef::FloatHistogram(value)), 11);
        value.positive_spans = Some(&spans);
        value.positive_buckets = Some(&buckets);
        assert_eq!(weight(SampleValueRef::FloatHistogram(value)), 13);
        value.negative_spans = Some(&spans);
        assert_eq!(weight(SampleValueRef::FloatHistogram(value)), 14);
        let integer = IntegerHistogram {
            positive_spans: Some(spans.to_vec()),
            positive_buckets: Some(vec![1, 1, 1, 1]),
            negative_spans: Some(spans.to_vec()),
            negative_buckets: Some(vec![1, 1, 1, 1]),
            custom_values: Some(vec![1.0, 2.0].into()),
            ..Default::default()
        };
        assert_eq!(
            weight(SampleValueRef::IntegerHistogram(integer.as_ref())),
            16
        );
        let mut custom = integer.as_ref();
        custom.schema = CUSTOM_BUCKETS_SCHEMA;
        assert_eq!(weight(SampleValueRef::IntegerHistogram(custom)), 14);
    }

    #[test]
    fn leases_follow_real_owners_and_isolate_queries() {
        let engine = EngineAccounting::default();
        let first = engine.query();
        let second = engine.query();
        let mut a = OwnedSamples::new(&first, 13);
        let b = OwnedSamples::new(&first, 1);
        assert_eq!(first.snapshot().statistics.samples.peak_samples, 14);
        a.resize(11);
        let c = OwnedSamples::new(&second, 2);
        assert_eq!(second.snapshot().statistics.samples.peak_samples, 2);
        drop((a, b, c));
        assert_eq!(first.current_samples(), 0);
        assert_eq!(second.current_samples(), 0);
        assert_eq!(first.snapshot().statistics.samples.peak_samples, 14);
    }

    #[test]
    fn transfers_preserve_peaks_and_unwind_releases_owned_values() {
        let account = EngineAccounting::default().query();
        let mut buffer = OwnedSamples::new(&account, 2);
        let point = OwnedSamples::new(&account, 1);
        let peak = account.snapshot().statistics.samples.peak_samples;
        buffer.absorb(point);
        assert_eq!(account.current_samples(), 3);
        assert_eq!(account.snapshot().statistics.samples.peak_samples, peak);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _point = ValueParts::new(crate::result::OwnedValue::Float(42.0), &account);
            let _buffer = buffer;
            panic!("injected owned-value unwind");
        }));
        assert!(outcome.is_err());
        assert_eq!(account.current_samples(), 0);
        assert_eq!(account.snapshot().statistics.samples.peak_samples, 4);
    }

    #[test]
    fn useful_reads_exclude_unused_instant_prefix_and_matrix_validation() {
        use crate::{
            instant_select::InstantWindow,
            matrix_reader::{self, MatrixWindow},
            storage::{Control, TimeBounds},
            test_storage::{GeneratedSeries, counter},
        };
        for page in [1, 17, 1024] {
            let account = EngineAccounting::default().query();
            let control = Control::default();
            let source = GeneratedSeries::new(1024, page, counter);
            let bounds = TimeBounds {
                start_ms: 0,
                end_ms: 1_024_000,
            };
            let point = crate::instant_statistics::select(
                &source,
                InstantWindow::single(100_000, 1, bounds),
                &control,
                &account,
            )
            .unwrap();
            assert!(point.is_some());
            assert_eq!(account.snapshot().statistics.samples.samples_read, 1);
            drop(point);
            let window = MatrixWindow {
                first_end_ms: 10_000,
                end_ms: 10_000,
                range_ms: 3000,
                buffer_ms: None,
                step_ms: 1000,
                data_bounds: bounds,
                statistics: false,
                reduce_delta: false,
                resume: None,
            };
            let mut count = 0;
            let acquisition = matrix_reader::acquire::<_, crate::storage::StorageError>(
                &source,
                Some(TimeBounds {
                    start_ms: 7001,
                    end_ms: 10_000,
                }),
                window,
                &control,
                &account,
                |_| {
                    count += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(count, 3);
            assert_eq!(acquisition.progress.sample_units, 3);
            assert_eq!(account.snapshot().statistics.samples.samples_read, 4);
            assert_eq!(account.current_samples(), 0);
            assert_eq!(
                source
                    .counters
                    .active
                    .load(std::sync::atomic::Ordering::Relaxed),
                0
            );
        }
    }

    #[derive(Default)]
    struct Discard {
        committed: Option<Statistics>,
        aborted: bool,
    }
    impl crate::result::OutputSink for Discard {
        fn begin(
            &mut self,
            _: crate::result::OutputHeader,
        ) -> Result<(), crate::accounting::AccountedVec<u8>> {
            Ok(())
        }
        fn chunk(
            &mut self,
            _: crate::result::OutputChunk<'_>,
        ) -> Result<(), crate::accounting::AccountedVec<u8>> {
            Ok(())
        }
        fn commit(
            &mut self,
            observations: &crate::accounting::QueryObservations,
        ) -> Result<(), crate::accounting::AccountedVec<u8>> {
            self.committed = Some(observations.statistics);
            Ok(())
        }
        fn abort(&mut self) {
            self.aborted = true;
        }
    }

    #[test]
    fn primary_logical_windows_exclude_output_replay_and_compose_subqueries() {
        use crate::{
            execution::{Engine, EngineOptions, QueryRequest},
            storage::Control,
            test_storage::FixtureProvider,
        };
        let instant = QueryRequest::Instant { time_ms: 3000 };
        let range = QueryRequest::Range {
            start_ms: 3000,
            end_ms: 5000,
            step_ms: 1000,
        };
        // Pinned engine.go:1919,2010-2063,2410,2617,2643 define primary window/invariant attribution.
        let cases = [
            ("42", instant, 0),
            ("\"text\"", instant, 0),
            ("sum(counter_total)", instant, 2),
            ("sum(counter_total)", range, 6),
            ("counter_total[3s]", instant, 6),
            ("sum(rate(counter_total[3s]))", instant, 6),
            ("sum(rate(counter_total[3s]))", range, 18),
            ("sum(abs(counter_total @ 3))", range, 6),
            ("sum(counter_total @ 3 + counter_total @ 4)", range, 12),
            ("sum(counter_total @ 3 + scalar(vector(time())))", range, 6),
            ("sum(rate(counter_total[3s] @ 3))", range, 18),
            ("sum_over_time(counter_total @ 3 [3s:1s] @ 3)", range, 18),
            ("counter_total[3s:1s]", instant, 6),
            ("sum_over_time(counter_total[3s:1s])", instant, 6),
            ("sum_over_time(counter_total[3s:1s])", range, 18),
            (
                "sum_over_time(sum_over_time(counter_total[2s:1s])[3s:1s])",
                instant,
                6,
            ),
        ];
        for page in [1, 17, 1024] {
            for (source, request, expected) in cases {
                let engine = Engine::default();
                let control = Control::default();
                let provider = FixtureProvider::new(20, page, 2);
                let query = engine
                    .compile(source.as_bytes(), EngineOptions::default(), &control)
                    .unwrap();
                let account = engine.accounting.query();
                let mut sink = Discard::default();
                let report =
                    query.execute_reported_in(&provider, request, &control, &mut sink, &account);
                assert!(report.result.is_ok(), "{source}: {:?}", report.result);
                let stats = report.observations.statistics;
                assert_eq!(
                    stats.samples.total_queryable_samples, expected,
                    "{source}, page{page}"
                );
                if expected == 0 {
                    assert_eq!(stats.samples.samples_read, 0);
                } else {
                    assert!(stats.samples.samples_read > 0);
                }
                assert_eq!(stats.timings.exec_queue_time, Duration::ZERO);
                assert!(stats.timings.exec_total_time >= stats.timings.eval_total_time);
                assert!(stats.timings.eval_total_time >= stats.timings.inner_eval_time);
                assert!(stats.timings.eval_total_time >= stats.timings.query_preparation_time);
                assert_eq!(sink.committed, Some(stats));
                assert!(!sink.aborted);
                assert_eq!(account.current_samples(), 0, "{source}");
                provider.assert_closed();
            }
        }
    }

    #[test]
    fn collected_sample_ownership_survives_execution_and_releases_with_result() {
        use crate::{
            execution::{Engine, EngineOptions, QueryRequest},
            storage::Control,
            test_storage::FixtureProvider,
        };
        let engine = Engine::default();
        let control = Control::default();
        let provider = FixtureProvider::new(20, 17, 2);
        let query = engine
            .compile(b"counter_total", EngineOptions::default(), &control)
            .unwrap();
        let account = engine.accounting.query();
        let report = query.collect_reported_in(
            &provider,
            QueryRequest::Range {
                start_ms: 3000,
                end_ms: 5000,
                step_ms: 1000,
            },
            &control,
            &account,
        );
        assert!(report.result.is_ok());
        assert_eq!(account.current_samples(), 6);
        assert!(report.observations.statistics.samples.peak_samples >= 6);
        drop(report);
        assert_eq!(account.current_samples(), 0);
        provider.assert_closed();
    }
}
