// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{result::OwnedValue, test_storage::FixtureProvider};

fn cardinality_work(expression: &[u8]) -> [u64; 2] {
    let mut work = [0; 2];
    for (position, count) in [128, 512].into_iter().enumerate() {
        let engine = Engine::default();
        let control = Control::default();
        let provider = FixtureProvider::new(1, 17, count);
        let query = engine
            .compile(expression, EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
        let result = report.result.as_ref().unwrap();
        assert_eq!(result.series.len(), count);
        let mut instances = std::collections::BTreeSet::new();
        for series in &*result.series {
            let instance = series
                .labels
                .0
                .iter()
                .find(|label| &*label.name == b"instance")
                .unwrap();
            assert!(instances.insert(instance.value.to_vec()));
            assert_eq!(series.points.len(), 1);
            assert_eq!(series.points[0].time_ms, 1000);
            let OwnedValue::Float(value) = series.points[0].value else {
                panic!("cardinality control must emit floats");
            };
            assert_eq!(value.to_bits(), 1.0f64.to_bits());
        }
        assert_eq!(instances.len(), count);
        work[position] = report.observations.work;
        provider.assert_closed();
        drop((report, query));
        engine.accounting.snapshot().assert_memory_released();
    }
    work
}

#[test]
fn grouped_sum_lookup_work_avoids_all_pairs() {
    let work = cardinality_work(b"sum by (instance) (counter_total)");
    assert!(work[1] <= work[0] * 6, "4x rows used work {work:?}");
}

#[test]
fn binary_matching_lookup_work_avoids_all_pairs() {
    let work = cardinality_work(b"counter_total / on (instance) counter_total");
    assert!(work[1] <= work[0] * 6, "4x rows used work {work:?}");
}

#[test]
fn unique_root_binary_replay_fetches_linear_samples() {
    fn point(_: usize) -> crate::value::Sample {
        crate::value::Sample {
            time_ms: 0,
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(1.0),
        }
    }
    let mut reads = Vec::new();
    for page_size in [1, 17, 1024] {
        for count in [16, 32, 64] {
            let engine = Engine::default();
            let control = Control::default();
            let mut provider = FixtureProvider::new(1, page_size, count);
            for series in &mut provider.series {
                series.source.sample = point;
            }
            let query = engine
                .compile(
                    b"counter_total / on (instance) counter_total",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 0,
                    end_ms: 1000,
                    step_ms: 1000,
                },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), count);
            assert!(report.annotations.is_empty());
            let mut instances = std::collections::BTreeSet::new();
            for series in &*result.series {
                assert_eq!(series.labels.0.len(), 1);
                assert_eq!(&*series.labels.0[0].name, b"instance");
                assert!(instances.insert(series.labels.0[0].value.to_vec()));
                let points: Vec<_> = series
                    .points
                    .iter()
                    .map(|point| {
                        let OwnedValue::Float(value) = point.value else {
                            panic!("float join");
                        };
                        (point.time_ms, value.to_bits())
                    })
                    .collect();
                assert_eq!(points, [(0, 1.0f64.to_bits()), (1000, 1.0f64.to_bits())]);
            }
            assert_eq!(
                instances,
                (0..count)
                    .map(|index| index.to_string().into_bytes())
                    .collect()
            );
            let observed = &report.observations;
            assert_eq!(observed.output_samples, 2 * count as u64);
            println!(
                "{{\"root_join_rows\":{count},\"page_size\":{page_size},\"fetched\":{},\"work\":{},\"workspace_peak\":{}}}",
                observed.samples_fetched,
                observed.work,
                observed.memory[MemoryKind::Workspace as usize].peak
            );
            reads.push((count, page_size, observed.samples_fetched));
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
    assert!(
        reads
            .iter()
            .all(|(count, _, fetched)| *fetched < 30 * *count as u64),
        "unique root join reads must scale with selected rows: {reads:?}"
    );
}

#[test]
fn unique_root_binary_replay_preserves_changing_membership_and_operand_positions() {
    fn point<const RIGHT: bool, const ID: usize>(index: usize) -> crate::value::Sample {
        let stale = f64::from_bits(crate::value::STALE_NAN_BITS);
        let left = [[1.0, 4.0, stale], [10.0, stale, 9.0], [10.0, 30.0, 5.0]];
        let right = [[2.0, 3.0, stale], [2.0, 2.0, 8.0], [20.0, 20.0, stale]];
        crate::value::Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(if RIGHT {
                right[ID][index]
            } else {
                left[ID][index]
            }),
        }
    }
    let cases = [
        (
            b"a - on(instance) b".as_slice(),
            [
                vec![(1000, -1.0f64), (2000, 1.0)],
                vec![(1000, 8.0), (3000, 1.0)],
                vec![(1000, -10.0), (2000, 10.0)],
            ],
        ),
        (
            b"a > on(instance) b".as_slice(),
            [
                vec![(2000, 4.0)],
                vec![(1000, 10.0), (3000, 9.0)],
                vec![(2000, 30.0)],
            ],
        ),
        (
            b"a > bool on(instance) b".as_slice(),
            [
                vec![(1000, 0.0), (2000, 1.0)],
                vec![(1000, 1.0), (3000, 1.0)],
                vec![(1000, 0.0), (2000, 1.0)],
            ],
        ),
    ];
    for page_size in [1, 17, 1024] {
        for delayed in [false, true] {
            for (expression, expected) in &cases {
                let mut provider = FixtureProvider::new(3, page_size, 6);
                provider.filter_selectors = true;
                let samples: [fn(usize) -> crate::value::Sample; 6] = [
                    point::<false, 0>,
                    point::<false, 1>,
                    point::<false, 2>,
                    point::<true, 2>,
                    point::<true, 1>,
                    point::<true, 0>,
                ];
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.labels = vec![
                        (
                            b"__name__".to_vec(),
                            if index < 3 { b"a" } else { b"b" }.to_vec(),
                        ),
                        (
                            b"instance".to_vec(),
                            (if index < 3 { index } else { 5 - index })
                                .to_string()
                                .into_bytes(),
                        ),
                    ];
                    series.source.sample = samples[index];
                }
                let engine = Engine::default();
                let control = Control::default();
                let query = engine
                    .compile(
                        expression,
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..EngineOptions::default()
                        },
                        &control,
                    )
                    .unwrap();
                let report = query.collect_reported(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 1000,
                        end_ms: 3000,
                        step_ms: 1000,
                    },
                    &control,
                );
                let result = report.result.as_ref().unwrap();
                assert_eq!(result.series.len(), 3);
                assert!(report.annotations.is_empty());
                for (id, series) in result.series.iter().enumerate() {
                    assert_eq!(series.labels.0.len(), 1);
                    assert_eq!(&*series.labels.0[0].name, b"instance");
                    assert_eq!(&*series.labels.0[0].value, id.to_string().as_bytes());
                    let points: Vec<_> = series
                        .points
                        .iter()
                        .map(|point| {
                            let OwnedValue::Float(value) = point.value else {
                                panic!("float pair");
                            };
                            (point.time_ms, value.to_bits())
                        })
                        .collect();
                    assert_eq!(
                        points,
                        expected[id]
                            .iter()
                            .map(|(time, value)| (*time, value.to_bits()))
                            .collect::<Vec<_>>(),
                        "{} page{page_size} delayed{delayed}",
                        String::from_utf8_lossy(expression)
                    );
                }
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn unique_root_binary_replay_keeps_raw_workspace_independent_of_history() {
    struct Discard {
        start_ms: i64,
        points: usize,
        committed: bool,
        aborted: bool,
    }
    impl OutputSink for Discard {
        fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
            assert_eq!(header.start_ms, self.start_ms);
            assert_eq!(header.end_ms, self.start_ms + 1000);
            Ok(())
        }
        fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
            let OutputChunk::Series { labels, points, .. } = chunk else {
                panic!("series output");
            };
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].name, b"instance");
            assert_eq!(points.len(), 1);
            assert_eq!(
                points[0].time_ms,
                self.start_ms + (self.points / 4) as i64 * 1000
            );
            let crate::value::SampleValueRef::Float(value) = points[0].value else {
                panic!("float output");
            };
            assert_eq!(value.to_bits(), 1.0f64.to_bits());
            self.points += 1;
            Ok(())
        }
        fn commit(&mut self, observations: &QueryObservations) -> Result<(), AccountedVec<u8>> {
            assert_eq!(self.points, 8);
            assert_eq!(observations.output_samples, 8);
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.aborted = true;
        }
    }
    for page_size in [1, 17, 1024] {
        let mut peaks = Vec::new();
        for history in [2048, 8192] {
            let provider = FixtureProvider::new(history, page_size, 4);
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(
                    b"counter_total / on(instance) counter_total",
                    EngineOptions {
                        lookback_ms: 1_000_000_000,
                        ..EngineOptions::default()
                    },
                    &control,
                )
                .unwrap();
            let start_ms = (history as i64 - 1) * 1000;
            let mut sink = Discard {
                start_ms,
                points: 0,
                committed: false,
                aborted: false,
            };
            let observations = query
                .execute(
                    &provider,
                    QueryRequest::Range {
                        start_ms,
                        end_ms: start_ms + 1000,
                        step_ms: 1000,
                    },
                    &control,
                    &mut sink,
                )
                .unwrap();
            assert!(sink.committed && !sink.aborted);
            assert_eq!(
                provider.series[0]
                    .source
                    .counters
                    .peak
                    .load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            peaks.push(
                [
                    MemoryKind::Workspace,
                    MemoryKind::ProviderHeld,
                    MemoryKind::BorrowedView,
                ]
                .map(|kind| {
                    let observed = observations.memory[kind as usize];
                    assert_eq!(observed.current, 0);
                    observed.peak
                }),
            );
            provider.assert_closed();
            drop(query);
            engine.accounting.snapshot().assert_memory_released();
        }
        assert_eq!(peaks[0], peaks[1], "page{page_size}");
    }
}
