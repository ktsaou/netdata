// SPDX-License-Identifier: GPL-3.0-or-later
//! Certified final range traversal keeps one source cursor and chronological row output.

use super::*;
use crate::{evaluator::ScheduledRow, query_scopes::Grid, result::OwnedLabels, storage::LabelRef};

pub(super) struct FinalSchedule<'a> {
    pub(super) scope: EvaluationScope<'a>,
    pub(super) rosters: &'a Rosters,
    pub(super) merge_root: bool,
    pub(super) cleanup_guard: &'a MergeGuard,
}

pub(super) fn emit_point<S: OutputSink>(
    sink: &mut S,
    series_id: usize,
    labels: &[LabelRef<'_>],
    point: SampleRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), ExecutionError> {
    sink.chunk_checked(
        OutputChunk::Series {
            series_id,
            labels,
            points: &[point],
        },
        control,
        account,
    )
    .map_err(|error| ExecutionError::output(error, account))
}

impl CompiledQuery {
    pub(super) fn try_scheduled_output<Q: QueryProvider, S: OutputSink>(
        &self,
        catalog: &Catalog<'_, Q>,
        schedule: FinalSchedule<'_>,
        control: &Control,
        account: &QueryAccounting,
        sink: &mut S,
    ) -> Result<bool, ExecutionError> {
        let FinalSchedule {
            scope,
            rosters,
            merge_root,
            cleanup_guard,
        } = schedule;
        let prepared = scope.prepared;
        let grid = Grid::from(prepared.evaluator_header());
        let root = self.expression.root;
        if self.value_type() != ValueType::Vector
            || prepared.header.value_type != ValueType::Matrix
            || !grid.forward_without_wrap()
            || rosters.scheduled_selector(root).is_none()
        {
            return Ok(false);
        }
        let machine = Machine::new(
            catalog,
            &self.expression,
            root,
            self.evaluation_options(grid.header(prepared.header), grid.end_ms, prepared),
            control,
            account,
            EvaluationMetadata {
                prepared,
                rosters,
                timestamp_selectors: &self.timestamp_selectors,
                data_bounds: scope.data_bounds,
                annotations: None,
                binary_preparation: None,
                histogram_preparation: None,
                info_preparation: None,
                count_preparation: None,
            },
        );
        let rows = rosters.descriptors(root);
        for descriptor in rows {
            control.check()?;
            account.work(1);
            if machine
                .scheduled_row(root, descriptor.ordinal, grid, false)?
                .is_none()
            {
                return Ok(false);
            }
            if self.options.delayed_name_removal {
                let cleaned = self.scheduled_labels(descriptor, control, account)?;
                let labels = cleaned.as_ref().unwrap_or(&descriptor.labels);
                if cleanup_guard.checks(
                    crate::label_hash::full(labels, control, account)?,
                    control,
                    account,
                )? {
                    return Ok(false);
                }
            }
        }
        let raw = crate::evaluator::raw_selector_shortcut(&self.expression, prepared, root);
        let mut identities = AccountedVec::new(account, MemoryKind::Workspace);
        if merge_root {
            // Existing ids follow the first productive tick, including filtered and delayed rows.
            for (position, descriptor) in rows.iter().enumerate() {
                let row = machine
                    .scheduled_row(root, descriptor.ordinal, grid, false)?
                    .expect("certified final scheduled row");
                let mut first = None;
                self.visit_final_schedule(&machine, catalog, row, grid, raw, |time, sample| {
                    if sample.is_some() {
                        first = Some(time);
                        return Ok(true);
                    }
                    Ok(false)
                })?;
                let Some(first) = first else {
                    return Ok(false);
                };
                identities.push((first, position));
            }
            crate::evaluator_order::sort(&mut identities, control, account, <(i64, usize)>::cmp)?;
        }
        for order in 0..rows.len() {
            control.check()?;
            account.work(1);
            let position = if merge_root {
                identities[order].1
            } else {
                order
            };
            let descriptor = &rows[position];
            let row = machine
                .scheduled_row(root, descriptor.ordinal, grid, false)?
                .expect("certified final scheduled row");
            let cleaned = self.scheduled_labels(descriptor, control, account)?;
            let labels = cleaned
                .as_ref()
                .unwrap_or(&descriptor.labels)
                .views_checked(account, control)?;
            let series_id = if merge_root {
                order
            } else {
                descriptor.ordinal
            };
            self.visit_final_schedule(&machine, catalog, row, grid, raw, |time, sample| {
                if let Some(sample) = sample {
                    emit_point(
                        sink,
                        series_id,
                        &labels,
                        SampleRef {
                            time_ms: time,
                            start_time_ms: 0,
                            value: sample.value,
                        },
                        control,
                        account,
                    )?;
                    account.output(1);
                }
                Ok(false)
            })?;
        }
        Ok(true)
    }

    fn scheduled_labels(
        &self,
        descriptor: &Descriptor,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<OwnedLabels>, EvaluationError> {
        if !self.options.delayed_name_removal || !descriptor.drop_name {
            return Ok(None);
        }
        let mut labels = descriptor
            .labels
            .copy_checked(account, MemoryKind::Workspace, control)?;
        drop_reserved(&mut labels, false, account, control)?;
        Ok(Some(labels))
    }

    fn visit_final_schedule<Q: QueryProvider>(
        &self,
        machine: &Machine<'_, '_, Q>,
        catalog: &Catalog<'_, Q>,
        row: ScheduledRow,
        grid: Grid,
        raw: bool,
        mut visit: impl FnMut(i64, Option<SampleRef<'_>>) -> Result<bool, ExecutionError>,
    ) -> Result<(), ExecutionError> {
        catalog
            .with_series(row.selector, row.physical_ordinal, |series| {
                if raw {
                    machine
                        .visit_raw_series(
                            row.selector,
                            row.physical_ordinal,
                            series,
                            grid,
                            None,
                            |time, _, sample| visit(time, sample),
                        )
                        .map(|_| ())
                } else {
                    machine.visit_scheduled_row(row, series, grid, None, visit)
                }
            })
            .expect("certified final physical row")
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::{test_storage::FixtureProvider, value::SampleValueRef};
    use std::sync::atomic::Ordering;

    #[derive(Default)]
    struct CounterSink {
        points: usize,
        committed: bool,
    }
    impl OutputSink for CounterSink {
        fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
            assert_eq!(header.value_type, ValueType::Matrix);
            Ok(())
        }
        fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
            let OutputChunk::Series { labels, points, .. } = chunk else {
                panic!("unexpected generated counter output");
            };
            assert_eq!(labels.len(), 3);
            for point in points {
                let SampleValueRef::Float(value) = point.value else {
                    panic!("unexpected generated counter value");
                };
                self.points += 1;
                assert_eq!(point.time_ms, self.points as i64 * 1000);
                assert_eq!(point.start_time_ms, 0);
                assert_eq!(value, self.points as f64);
            }
            Ok(())
        }
        fn commit(&mut self, _: &QueryObservations) -> Result<(), AccountedVec<u8>> {
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.committed = false;
        }
    }

    #[test]
    fn final_range_selector_does_not_revisit_earlier_ticks() {
        let provider = FixtureProvider::new(128, 17, 1);
        let counters = provider.series[0].source.counters.clone();
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(b"counter_total", EngineOptions::default(), &control)
            .unwrap();
        let mut sink = CounterSink::default();
        let report = query.execute_reported(
            &provider,
            QueryRequest::Range {
                start_ms: 1000,
                end_ms: 128000,
                step_ms: 1000,
            },
            &control,
            &mut sink,
        );
        assert!(report.result.is_ok(), "{:?}", report.result);
        assert!(sink.committed);
        assert_eq!(sink.points, 128);
        assert!(
            report.observations.samples_fetched <= 4 * 128,
            "final range fetched {} samples for 128 source points",
            report.observations.samples_fetched,
        );
        assert_eq!(counters.peak.load(Ordering::Relaxed), 1);
        provider.assert_closed();
        drop((report, query));
        engine.accounting.snapshot().assert_memory_released();
    }

    #[test]
    fn scheduled_maps_do_not_duplicate_large_label_payload_per_point() {
        struct PayloadSink<'a> {
            provider: &'a FixtureProvider,
            expected: &'a [(Vec<u8>, Vec<u8>)],
            count: usize,
            points: usize,
            committed: bool,
        }
        impl OutputSink for PayloadSink<'_> {
            fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
                assert_eq!(header.value_type, ValueType::Matrix);
                assert_eq!(header.start_ms, 1000);
                assert_eq!(header.end_ms, self.count as i64 * 1000);
                assert_eq!(header.step_ms, 1000);
                Ok(())
            }
            fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
                let OutputChunk::Series { labels, points, .. } = chunk else {
                    panic!("unexpected scheduled payload output");
                };
                assert_eq!(labels.len(), self.expected.len());
                for (label, (name, value)) in labels.iter().zip(self.expected) {
                    assert_eq!(label.name, name.as_slice());
                    assert_eq!(label.value, value.as_slice());
                }
                for point in points {
                    self.points += 1;
                    let SampleValueRef::Float(value) = point.value else {
                        panic!("expected generated float");
                    };
                    assert_eq!(point.time_ms, self.points as i64 * 1000);
                    assert_eq!(point.start_time_ms, 0);
                    assert_eq!(value.to_bits(), (self.points as f64).to_bits());
                }
                Ok(())
            }
            fn commit(&mut self, observations: &QueryObservations) -> Result<(), AccountedVec<u8>> {
                assert_eq!(self.points, self.count);
                assert_eq!(observations.output_samples, self.count as u64);
                self.provider.assert_closed();
                self.committed = true;
                Ok(())
            }
            fn abort(&mut self) {
                self.committed = false;
            }
        }

        const PAYLOAD: usize = 32 * 1024;
        let mut excessive = Vec::new();
        for page in [1, 17, 1024] {
            for delayed in [false, true] {
                for count in [64, 128] {
                    let mut provider = FixtureProvider::new(count, page, 1);
                    provider.series[0].labels.push((
                        b"payload".to_vec(),
                        (0..PAYLOAD).map(|index| index as u8).collect(),
                    ));
                    let expected = provider.series[0].labels[1..].to_vec();
                    let counters = provider.series[0].source.counters.clone();
                    let engine = Engine::default();
                    let control = Control::default();
                    let query = engine
                        .compile(
                            b"counter_total + 0",
                            EngineOptions {
                                lookback_ms: 1000,
                                delayed_name_removal: delayed,
                                ..EngineOptions::default()
                            },
                            &control,
                        )
                        .unwrap();
                    let mut sink = PayloadSink {
                        provider: &provider,
                        expected: &expected,
                        count,
                        points: 0,
                        committed: false,
                    };
                    let report = query.execute_reported(
                        &provider,
                        QueryRequest::Range {
                            start_ms: 1000,
                            end_ms: count as i64 * 1000,
                            step_ms: 1000,
                        },
                        &control,
                        &mut sink,
                    );
                    assert!(report.result.is_ok(), "{:?}", report.result);
                    assert!(sink.committed);
                    assert_eq!(counters.peak.load(Ordering::Relaxed), 1);
                    assert_eq!(report.observations.output_samples, count as u64);
                    assert_eq!(
                        report.observations.memory[MemoryKind::Workspace as usize].current,
                        0
                    );
                    let acquired =
                        report.observations.memory[MemoryKind::Workspace as usize].acquired;
                    let bound = ((3 * count + 16) * PAYLOAD + 256 * 1024) as u64;
                    eprintln!(
                        "scheduled-payload count{count} page{page} delayed{delayed} acquired{acquired} bound{bound}"
                    );
                    provider.assert_closed();
                    drop((report, query));
                    engine.accounting.snapshot().assert_memory_released();
                    if acquired > bound {
                        excessive.push((count, page, delayed, acquired, bound));
                    }
                }
            }
        }
        assert!(
            excessive.is_empty(),
            "completed scheduled points duplicate label payload: {excessive:?}"
        );
    }

    fn scaled<const FACTOR: i32>(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        sample.value = crate::value::SampleValue::Float((index + 1) as f64 * f64::from(FACTOR));
        sample
    }

    struct RowSink {
        points: [usize; 2],
        factors: [f64; 2],
        has_name: bool,
        committed: bool,
    }
    impl OutputSink for RowSink {
        fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
            assert_eq!(header.value_type, ValueType::Matrix);
            Ok(())
        }
        fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
            let OutputChunk::Series { labels, points, .. } = chunk else {
                panic!("unexpected generated row output");
            };
            assert_eq!(labels.len(), 2 + usize::from(self.has_name));
            assert_eq!(
                labels.iter().any(|label| label.name == b"__name__"),
                self.has_name
            );
            assert!(
                labels
                    .iter()
                    .any(|label| label.name == b"job" && label.value == b"worker")
            );
            let instance = labels
                .iter()
                .find(|label| label.name == b"instance")
                .unwrap();
            let instance: usize = std::str::from_utf8(instance.value)
                .unwrap()
                .parse()
                .unwrap();
            for point in points {
                let SampleValueRef::Float(value) = point.value else {
                    panic!("unexpected generated row value");
                };
                self.points[instance] += 1;
                assert_eq!(point.time_ms, self.points[instance] as i64 * 1000);
                assert_eq!(point.start_time_ms, 0);
                assert_eq!(
                    value.to_bits(),
                    (self.points[instance] as f64 * self.factors[instance]).to_bits()
                );
            }
            Ok(())
        }
        fn commit(&mut self, _: &QueryObservations) -> Result<(), AccountedVec<u8>> {
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.committed = false;
        }
    }

    #[test]
    fn final_range_maps_have_linear_reads_and_history_independent_workspace() {
        let cases = [
            ("counter_total", [-2.0, -3.0], true),
            ("-counter_total", [2.0, 3.0], false),
            ("round(abs(counter_total),0.5) + 0", [2.0, 3.0], false),
            ("counter_total + 2*time()", [0.0, -1.0], false),
        ];
        for page in [1, 17, 1024] {
            for delayed in [false, true] {
                for (expression, factors, has_name) in cases {
                    let mut peaks = Vec::new();
                    let mut work = Vec::new();
                    for count in [2048, 4096] {
                        let mut provider = FixtureProvider::new(count, page, 2);
                        provider.series[0].source.sample = scaled::<-2>;
                        provider.series[1].source.sample = scaled::<-3>;
                        provider.series.reverse();
                        let counters = provider.series[0].source.counters.clone();
                        let engine = Engine::default();
                        let control = Control::default();
                        let query = engine
                            .compile(
                                expression.as_bytes(),
                                EngineOptions {
                                    delayed_name_removal: delayed,
                                    ..EngineOptions::default()
                                },
                                &control,
                            )
                            .unwrap();
                        let mut sink = RowSink {
                            points: [0; 2],
                            factors,
                            has_name,
                            committed: false,
                        };
                        let report = query.execute_reported(
                            &provider,
                            QueryRequest::Range {
                                start_ms: 1000,
                                end_ms: count as i64 * 1000,
                                step_ms: 1000,
                            },
                            &control,
                            &mut sink,
                        );
                        assert!(
                            report.result.is_ok(),
                            "{expression} page{page} delayed{delayed}: {:?}",
                            report.result
                        );
                        assert!(sink.committed);
                        assert_eq!(sink.points, [count; 2]);
                        let observed = report.observations;
                        assert_eq!(observed.output_samples, 2 * count as u64);
                        assert_eq!(
                            observed.samples_fetched,
                            counters.samples_returned.load(Ordering::Relaxed) as u64
                        );
                        assert!(
                            observed.samples_fetched <= 12 * count as u64,
                            "{expression}: {} fetched",
                            observed.samples_fetched
                        );
                        assert!(
                            counters.opened.load(Ordering::Relaxed) <= 12,
                            "{expression}: too many cursors"
                        );
                        assert_eq!(counters.peak.load(Ordering::Relaxed), 1);
                        peaks.push(
                            [
                                MemoryKind::Workspace,
                                MemoryKind::ProviderHeld,
                                MemoryKind::BorrowedView,
                            ]
                            .map(|kind| observed.memory[kind as usize].peak),
                        );
                        work.push(observed.work);
                        provider.assert_closed();
                        drop((report.result, report.annotations, query));
                        engine.accounting.snapshot().assert_memory_released();
                    }
                    assert_eq!(
                        peaks[0], peaks[1],
                        "{expression} page{page} delayed{delayed}"
                    );
                    assert!(
                        work[1] * 10 <= work[0] * 23,
                        "{expression}: work grows faster than the query history"
                    );
                }
            }
        }
    }

    #[test]
    fn final_range_long_history_keeps_one_page_and_constant_workspace() {
        for page in [1, 17, 1024] {
            let mut peaks = Vec::new();
            for count in [10_000, 100_000] {
                let provider = FixtureProvider::new(count, page, 1);
                let counters = provider.series[0].source.counters.clone();
                let engine = Engine::default();
                let control = Control::default();
                let query = engine
                    .compile(b"counter_total", EngineOptions::default(), &control)
                    .unwrap();
                let mut sink = CounterSink::default();
                let report = query.execute_reported(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 1000,
                        end_ms: count as i64 * 1000,
                        step_ms: 1000,
                    },
                    &control,
                    &mut sink,
                );
                assert!(report.result.is_ok(), "{:?}", report.result);
                assert!(sink.committed);
                assert_eq!(sink.points, count);
                assert_eq!(report.observations.output_samples, count as u64);
                assert!(report.observations.samples_fetched <= 4 * count as u64);
                assert_eq!(counters.peak.load(Ordering::Relaxed), 1);
                peaks.push(
                    [
                        MemoryKind::Workspace,
                        MemoryKind::ProviderHeld,
                        MemoryKind::BorrowedView,
                    ]
                    .map(|kind| report.observations.memory[kind as usize].peak),
                );
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
            assert_eq!(peaks[0], peaks[1], "page{page}");
        }
    }

    fn late(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        if index < 2 {
            sample.value = crate::value::SampleValue::Float(f64::from_bits(0x7ff0_0000_0000_0002));
        }
        sample
    }

    #[derive(Default)]
    struct IdentitySink(Vec<(usize, usize, i64, u64)>);
    impl OutputSink for IdentitySink {
        fn begin(&mut self, _: OutputHeader) -> Result<(), AccountedVec<u8>> {
            Ok(())
        }
        fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
            let OutputChunk::Series {
                series_id,
                labels,
                points,
            } = chunk
            else {
                panic!("unexpected identity output");
            };
            let instance = labels
                .iter()
                .find(|label| label.name == b"instance")
                .unwrap();
            let instance = std::str::from_utf8(instance.value)
                .unwrap()
                .parse()
                .unwrap();
            for point in points {
                let SampleValueRef::Float(value) = point.value else {
                    panic!("unexpected identity value");
                };
                self.0
                    .push((instance, series_id, point.time_ms, value.to_bits()));
            }
            Ok(())
        }
        fn commit(&mut self, _: &QueryObservations) -> Result<(), AccountedVec<u8>> {
            Ok(())
        }
        fn abort(&mut self) {
            self.0.clear();
        }
    }

    #[test]
    fn scheduled_final_ids_follow_the_first_productive_tick() {
        for page in [1, 17, 1024] {
            for (expression, delayed, sign) in [
                ("counter_total", true, 1.0),
                ("-counter_total", false, -1.0),
            ] {
                let mut provider = FixtureProvider::new(4, page, 2);
                provider.series[0].source.sample = late;
                let engine = Engine::default();
                let control = Control::default();
                let query = engine
                    .compile(
                        expression.as_bytes(),
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..EngineOptions::default()
                        },
                        &control,
                    )
                    .unwrap();
                let mut sink = IdentitySink::default();
                let report = query.execute_reported(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 1000,
                        end_ms: 4000,
                        step_ms: 1000,
                    },
                    &control,
                    &mut sink,
                );
                assert!(report.result.is_ok(), "{:?}", report.result);
                sink.0.sort_by_key(|row| (row.0, row.2));
                let expected: Vec<_> = [(0, 1, 3..=4), (1, 0, 1..=4)]
                    .into_iter()
                    .flat_map(|(instance, id, ticks)| {
                        ticks.map(move |tick| {
                            (instance, id, tick * 1000, (sign * tick as f64).to_bits())
                        })
                    })
                    .collect();
                assert_eq!(sink.0, expected);
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn delayed_final_identity_collisions_keep_chronological_replay() {
        fn early(index: usize) -> crate::value::Sample {
            let mut sample = crate::test_storage::counter(index);
            if index >= 2 {
                sample.value =
                    crate::value::SampleValue::Float(f64::from_bits(0x7ff0_0000_0000_0002));
            }
            sample
        }
        for page in [1, 17, 1024] {
            let mut provider = FixtureProvider::new(4, page, 2);
            provider.series[0].source.sample = early;
            provider.series[1].source.sample = late;
            provider.series[1].labels[0].1 = b"other_total".to_vec();
            provider.series[1].labels[1].1 = b"0".to_vec();
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(
                    b"abs({job=\"worker\"})",
                    EngineOptions {
                        delayed_name_removal: true,
                        ..EngineOptions::default()
                    },
                    &control,
                )
                .unwrap();
            let (result, _) = query
                .collect(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 1000,
                        end_ms: 4000,
                        step_ms: 1000,
                    },
                    &control,
                )
                .unwrap();
            assert_eq!(result.series.len(), 1);
            assert!(
                !result.series[0]
                    .labels
                    .0
                    .iter()
                    .any(|label| &label.name[..] == b"__name__")
            );
            assert_eq!(result.series[0].points.len(), 4);
            for (index, point) in result.series[0].points.iter().enumerate() {
                let crate::result::OwnedValue::Float(value) = point.value else {
                    panic!("expected float");
                };
                assert_eq!(point.time_ms, (index + 1) as i64 * 1000);
                assert_eq!(value.to_bits(), ((index + 1) as f64).to_bits());
            }
            provider.assert_closed();
            drop((result, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
}
