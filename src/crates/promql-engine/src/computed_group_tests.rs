// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{computed_groups, result::OwnedValue, test_storage::FixtureProvider};

#[test]
fn mapped_subqueries_complete_each_cold_clock_before_reuse() {
    for page in [1, 17, 1024] {
        for inner in [1, 2, 4] {
            for body in [
                "100-sum by(instance)(counter_total)",
                "sum by(instance)(counter_total)/100",
                "sum by(instance)(counter_total)>10",
                "10<sum by(instance)(counter_total)",
                "sum by(instance)(counter_total)>bool 10",
                "-sum by(instance)(counter_total)",
                "abs(sum by(instance)(counter_total))",
                "-abs(sum by(instance)(counter_total))",
                "clamp(sum by(instance)(counter_total),4,20)",
                "100-sum by(instance)(rate(counter_total[3s]))",
            ] {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(25, page, 16);
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.labels[1].1 = (index / 2).to_string().into_bytes();
                    series.labels[2].1 = (index % 2).to_string().into_bytes();
                }
                let expression = format!("max_over_time((({body}))[4s:{inner}s])");
                let query = engine
                    .compile(expression.as_bytes(), EngineOptions::default(), &control)
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 20000,
                    step_ms: 2000,
                };
                let (expected, before) = {
                    let _fallback = crate::evaluator::disable_drained_child();
                    query.collect(&provider, request, &control).unwrap()
                };
                let (result, counts) = crate::evaluator::observe_aggregate_termination(None, || {
                    query.collect(&provider, request, &control)
                });
                let (actual, after) = result.unwrap();
                same_float_results(&actual, &expected);
                assert_eq!(
                    after.statistics.samples.total_queryable_samples,
                    before.statistics.samples.total_queryable_samples,
                    "{expression}, page={page}"
                );
                assert!(!actual.series.is_empty(), "{expression}");
                let final_phase = counts[1];
                assert!(final_phase[2] > 0, "cold interval path: {expression}");
                assert_eq!(
                    final_phase[2], final_phase[3],
                    "every cold clock reaches its real terminal step: {expression}"
                );
                assert_eq!(&final_phase[4..6], &[0, 0], "{expression}");
                assert!(
                    final_phase[2] <= 20 / inner as usize + 1,
                    "at most one full contributor scan per observed clock: {expression}, {counts:?}"
                );
                assert!(after.work < before.work, "{expression}, page={page}");
                drop((actual, expected, query));
                provider.assert_closed();
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn fixed_scalar_subquery_windows_preserve_results_and_reuse_completed_values() {
    fn sparse(index: usize) -> crate::value::Sample {
        let value = match index {
            9 => f64::from_bits(crate::value::STALE_NAN_BITS),
            16 => f64::from_bits(0x7ff8_0000_0000_0002),
            18 => -0.0,
            22 => f64::INFINITY,
            _ => index as f64,
        };
        crate::value::Sample {
            value: crate::value::SampleValue::Float(value),
            ..crate::test_storage::counter(index)
        }
    }
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for body in [
                "100-sum by(instance)(counter_total)",
                "sum by(instance)(counter_total)-100",
                "1/sum by(instance)(counter_total)",
                "sum by(instance)(counter_total)%3",
                "sum by(instance)(counter_total)^0.5",
                "sum by(instance)(counter_total)>20",
                "20<sum by(instance)(counter_total)",
                "20<bool sum by(instance)(counter_total)",
            ] {
                for reducer in ["avg_over_time", "quantile_over_time"] {
                    let expression = if reducer == "quantile_over_time" {
                        format!("quantile_over_time(0.9,(({body}))[4s:1s])")
                    } else {
                        format!("{reducer}((({body}))[4s:1s])")
                    };
                    let engine = Engine::default();
                    let control = Control::default();
                    let mut provider = FixtureProvider::new(29, page, 8);
                    for (index, series) in provider.series.iter_mut().enumerate() {
                        series.labels[1].1 = (index / 2).to_string().into_bytes();
                        series.labels[2].1 = (index % 2).to_string().into_bytes();
                        series.source.sample = sparse;
                    }
                    let query = engine
                        .compile(
                            expression.as_bytes(),
                            EngineOptions {
                                delayed_name_removal: delayed,
                                ..Default::default()
                            },
                            &control,
                        )
                        .unwrap();
                    let request = QueryRequest::Range {
                        start_ms: 4000,
                        end_ms: 28000,
                        step_ms: 1000,
                    };
                    let (ordinary, _) = computed_groups::observe(true, || {
                        query.collect(&provider, request, &control)
                    });
                    let (reused, hits) = computed_groups::observe(false, || {
                        query.collect(&provider, request, &control)
                    });
                    let (expected, before) = ordinary.unwrap();
                    let (actual, after) = reused.unwrap();
                    same_float_results(&actual, &expected);
                    assert_eq!(
                        after.statistics.samples.total_queryable_samples,
                        before.statistics.samples.total_queryable_samples,
                        "{expression}, page={page}, delayed={delayed}"
                    );
                    assert!(
                        hits[17] + hits[18] > 0,
                        "mapped windows must actually reuse values: {expression}, {hits:?}"
                    );
                    drop((actual, expected, query));
                    provider.assert_closed();
                    engine.accounting.snapshot().assert_memory_released();
                }
            }
        }
    }
}

#[test]
fn fixed_scalar_windows_preserve_name_projection_and_native_fallback() {
    fn mixed(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        if index.is_multiple_of(3) {
            sample.value =
                crate::value::SampleValue::FloatHistogram(crate::histogram::FloatHistogram {
                    count: index as f64 + 1.0,
                    sum: index as f64,
                    ..Default::default()
                });
        }
        sample
    }
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for mode in 0..3 {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(10, page, 4);
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.labels[1].1 = (index / 2).to_string().into_bytes();
                    series.labels[2].1 = (index % 2).to_string().into_bytes();
                    if mode == 1 && index % 2 == 1 {
                        series.labels[0].1 = b"counter_other".to_vec();
                    }
                    if mode == 2 {
                        series.source.sample = mixed;
                    }
                }
                let query = engine
                    .compile(
                        br#"avg_over_time((100-sum by(__name__,instance)({__name__=~"counter_.*"}))[3s:1s])"#,
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..Default::default()
                        },
                        &control,
                    )
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 3000,
                    end_ms: 9000,
                    step_ms: 1000,
                };
                let (ordinary, _) =
                    computed_groups::observe(true, || query.collect(&provider, request, &control));
                let ((reused, hits), progress_loads, progress_saves) =
                    crate::derived_progress::observe(false, || {
                        computed_groups::observe(false, || {
                            query.collect(&provider, request, &control)
                        })
                    });
                match (ordinary, reused) {
                    (Ok((expected, _)), Ok((actual, _))) => {
                        same_float_results(&actual, &expected);
                    }
                    (Err(expected), Err(actual)) => {
                        assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
                    }
                    _ => panic!("mapped outcome changed: mode={mode}, delayed={delayed}"),
                }
                if mode == 0 {
                    assert!(
                        hits[17] > 0,
                        "projected labels must permit unambiguous reuse"
                    );
                }
                if mode == 2 {
                    assert_eq!((progress_loads, progress_saves), (0, 0));
                    assert_eq!(
                        hits[17] + hits[18],
                        0,
                        "mixed history retains ordinary replay"
                    );
                }
                drop(query);
                provider.assert_closed();
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn phase_changed_native_inputs_revoke_mapped_window_progress() {
    thread_local! { static FINAL_NATIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            FINAL_NATIVE.set(false);
        }
    }
    fn changing(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        if index == 0 && FINAL_NATIVE.get() {
            sample.value =
                crate::value::SampleValue::FloatHistogram(crate::histogram::FloatHistogram {
                    count: 1.0,
                    sum: 1.0,
                    ..Default::default()
                });
        }
        sample
    }
    let _restore = Restore;
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for sources in [1, 2] {
                for operator in ["sum", "count", "max"] {
                    let run = |disabled| {
                        FINAL_NATIVE.set(false);
                        let engine = Engine::default();
                        let control = Control::default();
                        let mut provider = FixtureProvider::new(9, page, sources);
                        for (index, series) in provider.series.iter_mut().enumerate() {
                            series.labels[1].1 = b"shared".to_vec();
                            series.labels[2].1 = index.to_string().into_bytes();
                        }
                        provider.series[sources - 1].source.sample = changing;
                        let query = engine.compile(
                            format!("avg_over_time((100-{operator} by(instance)(counter_total))[4s:1s])").as_bytes(),
                            EngineOptions { delayed_name_removal: delayed, ..Default::default() },
                            &control,
                        ).unwrap();
                        let account = query.execution_accounting();
                        let ((values, loads, saves), _) =
                            computed_groups::observe(disabled, || {
                                let header = query
                                    .header(QueryRequest::Range {
                                        start_ms: 4000,
                                        end_ms: 8000,
                                        step_ms: 1000,
                                    })
                                    .unwrap();
                                let prepared = Prepared::new(
                                    &query.expression,
                                    &query.preprocessing,
                                    header,
                                    query.options.no_step_subquery_interval_ms,
                                    &control,
                                    &account,
                                )
                                .unwrap();
                                let header = prepared.evaluator_header();
                                prepared.activate(header, &control, &account).unwrap();
                                let options =
                                    query.evaluation_options(header, header.start_ms, &prepared);
                                let bounds = crate::query_bounds::find(
                                    &query.expression,
                                    options,
                                    &prepared,
                                    &control,
                                    &account,
                                )
                                .unwrap();
                                let scope = provider.open(bounds, &control).unwrap();
                                let catalog = Catalog::new(
                                    &scope,
                                    &query.expression,
                                    &prepared,
                                    options,
                                    query.options.no_step_subquery_interval_ms,
                                    &control,
                                    &account,
                                )
                                .unwrap();
                                let rosters = query
                                    .prepare_rosters(
                                        &catalog,
                                        &prepared,
                                        bounds,
                                        &control,
                                        &account,
                                        &mut Annotations::default(),
                                    )
                                    .unwrap();
                                FINAL_NATIVE.set(true);
                                crate::derived_progress::observe(false, || {
                                    let mut values = Vec::new();
                                    for index in 0..5 {
                                        let time_ms = header.start_ms + index * header.step_ms;
                                        let mut machine = Machine::new(
                                            &catalog,
                                            &query.expression,
                                            query.expression.root,
                                            query.evaluation_options(header, time_ms, &prepared),
                                            &control,
                                            &account,
                                            EvaluationMetadata {
                                                prepared: &prepared,
                                                rosters: &rosters,
                                                timestamp_selectors: &query.timestamp_selectors,
                                                data_bounds: bounds,
                                                annotations: None,
                                                binary_preparation: None,
                                                histogram_preparation: None,
                                                info_preparation: None,
                                                count_preparation: None,
                                            },
                                        );
                                        machine.set_step_index(index);
                                        while let Some(sample) = machine.next().unwrap() {
                                            let OwnedValue::Float(value) = sample.value else {
                                                panic!("Float range result")
                                            };
                                            values.push((time_ms, sample.ordinal, value.to_bits()));
                                        }
                                    }
                                    values
                                })
                            });
                        provider.assert_closed();
                        account.snapshot().assert_memory_released();
                        drop((account, query));
                        engine.accounting.snapshot().assert_memory_released();
                        (values, loads, saves)
                    };
                    let (expected, _, _) = run(true);
                    let (actual, loads, saves) = run(false);
                    assert_eq!(
                        actual, expected,
                        "{operator}, sources={sources}, page={page}, delayed={delayed}"
                    );
                    assert!(!actual.is_empty());
                    assert_eq!(
                        (loads, saves),
                        (0, 0),
                        "native input revokes mapped progress: {operator}, sources={sources}, page={page}, delayed={delayed}"
                    );
                }
            }
        }
    }
}

#[test]
fn grouped_subqueries_finish_each_cold_producer_before_reuse() {
    for sources in [32, 128, 512] {
        let engine = Engine::default();
        let control = Control::default();
        let mut provider = FixtureProvider::new(21, 1024, sources);
        for (index, series) in provider.series.iter_mut().enumerate() {
            series.labels[1].1 = (index / 2).to_string().into_bytes();
            series.labels[2].1 = (index % 2).to_string().into_bytes();
        }
        let query = engine
            .compile(
                b"avg_over_time((sum by(instance)(counter_total))[4s:1s])",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let (result, counts) = crate::evaluator::observe_aggregate_termination(None, || {
            query.collect(
                &provider,
                QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 19000,
                    step_ms: 1000,
                },
                &control,
            )
        });
        let (result, resources) = result.unwrap();
        assert_eq!(result.series.len(), sources / 2);
        assert!(result.series.iter().all(|series| series.points.len() == 16));
        for series in &result.series {
            for point in &series.points {
                let OwnedValue::Float(value) = point.value else {
                    panic!("Float result")
                };
                assert_eq!(
                    value.to_bits(),
                    (2.0 * (point.time_ms / 1000) as f64 - 3.0).to_bits()
                );
            }
        }
        assert_eq!(counts[1][2], 19, "one cold producer per observed clock");
        assert_eq!(
            counts[1][3], 19,
            "every cold producer reaches its actual terminal step"
        );
        assert_eq!(
            &counts[1][4..6],
            &[0, 0],
            "no completed cold child is discarded"
        );
        assert_eq!(
            counts[1][6],
            19 * sources,
            "cold input delivery grows linearly with source count"
        );
        eprintln!(
            "terminal sources={sources}, preparation={:?}, final={:?}, work={}, fetched={}",
            counts[0], counts[1], resources.work, resources.samples_fetched
        );
        drop((result, query));
        provider.assert_closed();
        engine.accounting.snapshot().assert_memory_released();
    }
}

#[test]
fn drained_aggregate_terminal_cancellation_prevents_publication() {
    for page in [1, 17, 1024] {
        let engine = Engine::default();
        let control = Control::default();
        let provider = FixtureProvider::new(21, page, 4);
        let query = engine
            .compile(
                b"avg_over_time((sum by(instance)(counter_total))[4s:1s])",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let (report, counts) = crate::evaluator::observe_aggregate_termination(Some(4000), || {
            query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 19000,
                    step_ms: 1000,
                },
                &control,
            )
        });
        assert!(matches!(
            report.result,
            Err(ExecutionError::Evaluation(EvaluationError::Interrupted(
                Interrupted::Cancelled
            )))
        ));
        assert_eq!(counts[1][2], 4, "only the first four cold clocks ran");
        assert_eq!(
            counts[1][3], 3,
            "cancelled endpoint cannot complete its interval"
        );
        assert_eq!(report.observations.output_samples, 0);
        drop((report, query));
        provider.assert_closed();
        engine.accounting.snapshot().assert_memory_released();
    }
}

#[test]
fn mapped_aggregate_terminal_cancellation_prevents_publication() {
    for page in [1, 17, 1024] {
        for body in [
            "100-sum by(instance)(counter_total)",
            "abs(sum by(instance)(counter_total))",
            "-abs(sum by(instance)(counter_total))",
        ] {
            let engine = Engine::default();
            let control = Control::default();
            let provider = FixtureProvider::new(21, page, 4);
            let expression = format!("max_over_time((({body}))[4s:1s])");
            let query = engine
                .compile(expression.as_bytes(), EngineOptions::default(), &control)
                .unwrap();
            let (report, counts) = crate::evaluator::observe_aggregate_termination(Some(4000), || {
                query.collect_reported(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 4000,
                        end_ms: 19000,
                        step_ms: 2000,
                    },
                    &control,
                )
            });
            assert!(matches!(
                report.result,
                Err(ExecutionError::Evaluation(EvaluationError::Interrupted(
                    Interrupted::Cancelled
                )))
            ));
            assert_eq!(counts[1][2], 4, "{expression}");
            assert_eq!(counts[1][3], 3, "cancelled endpoint: {expression}");
            assert_eq!(report.observations.output_samples, 0);
            drop((report, query));
            provider.assert_closed();
            engine.accounting.snapshot().assert_memory_released();
        }
    }
}

pub(super) fn same_float_results(actual: &CollectedResult, expected: &CollectedResult) {
    assert_eq!(
        format!("{:?}", actual.header),
        format!("{:?}", expected.header)
    );
    assert_eq!(actual.series.len(), expected.series.len());
    for (actual, expected) in actual.series.iter().zip(expected.series.iter()) {
        assert_eq!(actual.labels.0.len(), expected.labels.0.len());
        for (actual, expected) in actual.labels.0.iter().zip(expected.labels.0.iter()) {
            assert_eq!(&*actual.name, &*expected.name);
            assert_eq!(&*actual.value, &*expected.value);
        }
        assert_eq!(actual.points.len(), expected.points.len());
        for (actual, expected) in actual.points.iter().zip(expected.points.iter()) {
            assert_eq!(actual.time_ms, expected.time_ms);
            let (OwnedValue::Float(actual), OwnedValue::Float(expected)) =
                (&actual.value, &expected.value)
            else {
                panic!("float result")
            };
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    }
    assert_eq!(
        format!("{:?}", actual.annotations.iter().collect::<Vec<_>>()),
        format!("{:?}", expected.annotations.iter().collect::<Vec<_>>())
    );
}

#[test]
fn partial_group_producers_publish_only_finished_values_and_explicit_absence() {
    use crate::value::{STALE_NAN_BITS, Sample, SampleValue};
    thread_local! { static FINAL_KIND: std::cell::Cell<u8> = const { std::cell::Cell::new(0) }; }
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            FINAL_KIND.set(0);
        }
    }
    fn last(index: usize) -> Sample {
        let mut sample = crate::test_storage::counter(index);
        if index == 1 {
            sample.value = match FINAL_KIND.get() {
                1 => SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
                2 => SampleValue::FloatHistogram(crate::histogram::FloatHistogram {
                    count: 1.0,
                    sum: 1.0,
                    ..Default::default()
                }),
                _ => sample.value,
            };
        }
        sample
    }
    let _restore = Restore;
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for kind in [1, 2] {
                FINAL_KIND.set(0);
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(3, page, 4);
                provider.series[3].source.sample = last;
                let query = engine
                    .compile(
                        b"sum by(instance)(counter_total)/100",
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..Default::default()
                        },
                        &control,
                    )
                    .unwrap();
                let account = query.execution_accounting();
                {
                    let header = query
                        .header(QueryRequest::Range {
                            start_ms: 1000,
                            end_ms: 2000,
                            step_ms: 1000,
                        })
                        .unwrap();
                    let prepared = Prepared::new(
                        &query.expression,
                        &query.preprocessing,
                        header,
                        query.options.no_step_subquery_interval_ms,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let header = prepared.evaluator_header();
                    prepared.activate(header, &control, &account).unwrap();
                    let options = query.evaluation_options(header, 2000, &prepared);
                    let bounds = crate::query_bounds::find(
                        &query.expression,
                        options,
                        &prepared,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let scope = provider.open(bounds, &control).unwrap();
                    let catalog = Catalog::new(
                        &scope,
                        &query.expression,
                        &prepared,
                        options,
                        query.options.no_step_subquery_interval_ms,
                        &control,
                        &account,
                    )
                    .unwrap();
                    let rosters = query
                        .prepare_rosters(
                            &catalog,
                            &prepared,
                            bounds,
                            &control,
                            &account,
                            &mut Annotations::default(),
                        )
                        .unwrap();
                    let node = (0..query.expression.arena.nodes.len())
                        .map(NodeId)
                        .find(|node| rosters.groups(*node).is_some())
                        .unwrap();
                    assert_eq!(rosters.groups(node).unwrap().len(), 4);
                    let key = crate::vector_matching::ReplayKey::new(
                        options,
                        1,
                        prepared.lookback_ns(options.lookback_ms),
                        bounds,
                    );
                    let cell = |ordinal| {
                        catalog
                            .with_computed_group(node, &control, &account, |store| {
                                store.read(key, 2000, ordinal, &control, &account)
                            })
                            .unwrap()
                            .unwrap()
                    };
                    let machine = || {
                        let mut machine = Machine::new(
                            &catalog,
                            &query.expression,
                            node,
                            options,
                            &control,
                            &account,
                            EvaluationMetadata {
                                prepared: &prepared,
                                rosters: &rosters,
                                timestamp_selectors: &query.timestamp_selectors,
                                data_bounds: bounds,
                                annotations: None,
                                binary_preparation: None,
                                histogram_preparation: None,
                                info_preparation: None,
                                count_preparation: None,
                            },
                        );
                        machine.bypass_root_order();
                        machine
                    };
                    FINAL_KIND.set(kind);
                    let mut partial = machine();
                    assert_eq!(partial.next().unwrap().unwrap().ordinal, 0);
                    assert!(matches!(cell(0), computed_groups::Cell::Present(_)));
                    for ordinal in 1..4 {
                        assert!(matches!(cell(ordinal), computed_groups::Cell::Unknown));
                    }
                    drop(partial);
                    let collect = || {
                        let mut complete = machine();
                        let mut output = Vec::new();
                        while let Some(sample) = complete.next().unwrap() {
                            output.push((
                                sample.ordinal,
                                match sample.value {
                                    OwnedValue::Float(value) => Some(value.to_bits()),
                                    OwnedValue::Histogram(_) => None,
                                },
                            ));
                        }
                        output
                    };
                    let expected = collect();
                    assert_eq!(expected.len(), if kind == 1 { 3 } else { 4 });
                    for ordinal in 0..3 {
                        assert!(matches!(cell(ordinal), computed_groups::Cell::Present(_)));
                    }
                    assert!(matches!(
                        (kind, cell(3)),
                        (1, computed_groups::Cell::Absent) | (2, computed_groups::Cell::Unknown)
                    ));
                    assert_eq!(
                        catalog
                            .with_computed_group(node, &control, &account, |store| {
                                Ok(store.has_only_float_inputs())
                            })
                            .unwrap(),
                        Some(kind == 1),
                        "native input revokes mapped admission without deleting completed cells"
                    );
                    let (actual, hits) = computed_groups::observe(false, collect);
                    assert_eq!(actual, expected);
                    if kind == 1 {
                        assert_eq!(hits[5], 0, "complete All read avoids selector visits");
                    } else {
                        assert!(
                            hits[5] > 0,
                            "native output keeps the ordinary aggregate path"
                        );
                    }
                    control.cancellation.cancel();
                    assert!(machine().next().is_err());
                }
                provider.assert_closed();
                account.snapshot().assert_memory_released();
                drop((account, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn computed_selector_hits_validate_current_labels_and_control_before_reuse() {
    use crate::{
        storage::LabelRef,
        test_storage::{observe_label_reads, transform_fixture_labels},
    };
    use std::sync::atomic::Ordering;
    fn nonmatching(label: LabelRef<'_>) -> LabelRef<'_> {
        if label.name == b"job" {
            LabelRef {
                value: b"other",
                ..label
            }
        } else {
            label
        }
    }
    fn duplicate(label: LabelRef<'_>) -> LabelRef<'_> {
        if label.name == b"job" {
            LabelRef {
                name: b"instance",
                ..label
            }
        } else {
            label
        }
    }
    let engine = Engine::default();
    let control = Control::default();
    let provider = FixtureProvider::new(4, 1, 2);
    let query = engine
        .compile(
            b"sum by(instance)(counter_total{job=\"worker\"})",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    let account = query.execution_accounting();
    {
        let header = query
            .header(QueryRequest::Range {
                start_ms: 1000,
                end_ms: 2000,
                step_ms: 1000,
            })
            .unwrap();
        let prepared = Prepared::new(
            &query.expression,
            &query.preprocessing,
            header,
            query.options.no_step_subquery_interval_ms,
            &control,
            &account,
        )
        .unwrap();
        let header = prepared.evaluator_header();
        prepared.activate(header, &control, &account).unwrap();
        let options = query.evaluation_options(header, header.start_ms, &prepared);
        let bounds =
            crate::query_bounds::find(&query.expression, options, &prepared, &control, &account)
                .unwrap();
        let scope = provider.open(bounds, &control).unwrap();
        let catalog = Catalog::new(
            &scope,
            &query.expression,
            &prepared,
            options,
            query.options.no_step_subquery_interval_ms,
            &control,
            &account,
        )
        .unwrap();
        let rosters = query
            .prepare_rosters(
                &catalog,
                &prepared,
                bounds,
                &control,
                &account,
                &mut Annotations::default(),
            )
            .unwrap();
        let selector = NodeId(
            query
                .expression
                .arena
                .nodes
                .iter()
                .position(|node| matches!(node.expression, Expr::Selector(_)))
                .unwrap(),
        );
        let evaluate = || {
            let mut machine = Machine::new(
                &catalog,
                &query.expression,
                selector,
                options,
                &control,
                &account,
                EvaluationMetadata {
                    prepared: &prepared,
                    rosters: &rosters,
                    timestamp_selectors: &query.timestamp_selectors,
                    data_bounds: bounds,
                    annotations: None,
                    binary_preparation: None,
                    histogram_preparation: None,
                    info_preparation: None,
                    count_preparation: None,
                },
            );
            machine.restrict_one(0);
            machine.next().map(|sample| sample.is_some())
        };
        assert!(evaluate().unwrap());
        let before = provider.series[0]
            .source
            .counters
            .opened
            .load(Ordering::Relaxed);
        let (warm, hits) = computed_groups::observe(false, evaluate);
        assert!(warm.unwrap());
        assert_eq!(hits[5], 1, "the warm selector cell was used");
        let (nonmatch, reads) = observe_label_reads(&control, None, || {
            transform_fixture_labels(nonmatching, evaluate)
        });
        assert!(!nonmatch.unwrap());
        assert!(reads > 0);
        assert!(matches!(
            transform_fixture_labels(duplicate, evaluate),
            Err(EvaluationError::Storage(StorageError::Invalid(_)))
        ));
        let (cancelled, reads) = observe_label_reads(&control, Some(1), evaluate);
        assert!(matches!(
            cancelled,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                | Err(EvaluationError::Storage(StorageError::Interrupted(
                    Interrupted::Cancelled
                )))
        ));
        assert_eq!(reads, 1);
        assert_eq!(
            provider.series[0]
                .source
                .counters
                .opened
                .load(Ordering::Relaxed),
            before
        );
    }
    provider.assert_closed();
    account.snapshot().assert_memory_released();
    drop((account, query));
    engine.accounting.snapshot().assert_memory_released();
}

#[test]
fn computed_selector_points_preserve_mixed_values_source_clocks_and_logical_statistics() {
    fn sample(index: usize) -> crate::value::Sample {
        use crate::{
            histogram::FloatHistogram,
            value::{STALE_NAN_BITS, Sample, SampleValue},
        };
        Sample {
            time_ms: index as i64 * 1000 + 137,
            start_time_ms: index as i64 * 1000 - 71,
            value: match index % 11 {
                0 => SampleValue::Float(f64::from_bits(STALE_NAN_BITS)),
                1 => SampleValue::FloatHistogram(FloatHistogram {
                    count: 2.0,
                    sum: 3.0,
                    ..FloatHistogram::default()
                }),
                2 => SampleValue::Float(f64::from_bits(0x7ff8_0000_0000_0001)),
                3 => SampleValue::Float(-0.0),
                _ => SampleValue::Float(index as f64),
            },
        }
    }
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for expression in [
                "max by(instance)(timestamp((counter_total)))",
                "time()-max by(instance)(timestamp((counter_total)))",
                "time()-max by(instance)(start_timestamp(counter_total offset 500ms))",
                "max_over_time((max by(instance)(timestamp(counter_total)))[4s:1s])",
                "sum by(instance)(counter_total)>bool 0",
            ] {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(64, page, 8);
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.source.sample = sample;
                    series.labels[1].1 = (index / 2).to_string().into_bytes();
                    series.labels[2].1 = (index % 2).to_string().into_bytes();
                }
                let mut options = EngineOptions {
                    delayed_name_removal: delayed,
                    start_timestamps: true,
                    lookback_ms: 1200,
                    ..EngineOptions::default()
                };
                options.parser.experimental_functions = true;
                let query = engine
                    .compile(expression.as_bytes(), options, &control)
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 62000,
                    step_ms: 1000,
                };
                let (fallback, _) =
                    computed_groups::observe(true, || query.collect(&provider, request, &control));
                let (computed, hits) =
                    computed_groups::observe(false, || query.collect(&provider, request, &control));
                let (expected, before) = fallback.unwrap();
                let (actual, after) = computed.unwrap();
                same_float_results(&actual, &expected);
                assert_eq!(
                    after.statistics.samples.total_queryable_samples,
                    before.statistics.samples.total_queryable_samples,
                    "{expression}"
                );
                assert!(
                    hits[4] > 0,
                    "actual selector preparation reuse: {expression}"
                );
                assert_eq!(after.temporary_storage.extent.current, 0);
                provider.assert_closed();
                drop((actual, expected, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn completed_windows_materialize_exact_nan_and_zero_bits_for_temporal_sort() {
    fn point(index: usize) -> crate::value::Sample {
        let bits = [
            0x7ff8_0000_0000_0001,
            0x8000_0000_0000_0000,
            0,
            0x3ff0_0000_0000_0000,
            0x7ff8_0000_0000_0002,
        ][index % 5];
        crate::value::Sample {
            value: crate::value::SampleValue::Float(f64::from_bits(bits)),
            ..crate::test_storage::counter(index)
        }
    }
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            let engine = Engine::default();
            let control = Control::default();
            let mut provider = FixtureProvider::new(22, page, 2);
            for series in &mut provider.series {
                series.source.sample = point;
            }
            let query = engine
                .compile(
                    b"quantile_over_time(.5,(min by(instance)(counter_total))[16s:1s])",
                    EngineOptions {
                        delayed_name_removal: delayed,
                        ..Default::default()
                    },
                    &control,
                )
                .unwrap();
            let request = QueryRequest::Range {
                start_ms: 16000,
                end_ms: 20000,
                step_ms: 1000,
            };
            let (fallback, _) =
                computed_groups::observe(true, || query.collect(&provider, request, &control));
            let (cached, hits) =
                computed_groups::observe(false, || query.collect(&provider, request, &control));
            let (expected, before) = fallback.unwrap();
            let (actual, after) = cached.unwrap();
            same_float_results(&actual, &expected);
            assert!(hits[18] > 0, "actual completed-window sort materialization");
            assert_eq!(
                after.statistics.samples.total_queryable_samples,
                before.statistics.samples.total_queryable_samples
            );
            assert_eq!(after.temporary_storage.extent.current, 0);
            provider.assert_closed();
            drop((actual, expected, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn computed_groups_reuse_final_cells_and_preserve_complete_fallback_results() {
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for expression in [
                "sum by(instance)(counter_total)/100",
                "time()-max by(instance)(timestamp(counter_total))",
                "sum by(instance)(counter_total)/on() group_left sum(counter_total)",
                "quantile_over_time(.95,(((sum by(instance)(counter_total))))[4s:1s])",
            ] {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(34, page, 32);
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.labels[1].1 = (index / 2).to_string().into_bytes();
                    series.labels[2].1 = (index % 2).to_string().into_bytes();
                }
                let query = engine
                    .compile(
                        expression.as_bytes(),
                        EngineOptions {
                            delayed_name_removal: delayed,
                            start_timestamps: delayed,
                            ..EngineOptions::default()
                        },
                        &control,
                    )
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 32000,
                    step_ms: 1000,
                };
                let (ordinary, _) =
                    computed_groups::observe(true, || query.collect(&provider, request, &control));
                let (expected, before) = ordinary.unwrap();
                let (computed, counts) =
                    computed_groups::observe(false, || query.collect(&provider, request, &control));
                let (actual, after) = computed.unwrap();
                eprintln!(
                    "{page} {delayed} {expression}: {counts:?}, fetched {} -> {}",
                    before.samples_fetched, after.samples_fetched
                );
                same_float_results(&actual, &expected);
                assert!(counts[0] > 0, "preparation reuse: {expression}");
                if expression.starts_with("time()") {
                    assert!(
                        counts[5] <= 4 * 32 * 29,
                        "each freshness group must avoid visiting all source cells: {counts:?}"
                    );
                }
                if expression.starts_with("quantile_over_time") {
                    assert!(
                        counts[17] > 0,
                        "completed windows must actually be consumed: {counts:?}"
                    );
                }
                if expression.contains("group_left") {
                    assert!(
                        counts[1] < 10 * 16 * 29,
                        "ratio matching must not rediscover all finished groups per output: {counts:?}"
                    );
                    assert_eq!(
                        counts[5],
                        counts[19] + counts[20] + counts[21],
                        "selector hits must be seeds, pending consumption or overlapping prefetch probes"
                    );
                    assert!(counts[1] > 0, "final reuse: {expression}");
                    assert!(counts[3] > 0, "fresh final production: {expression}");
                }
                assert!(after.samples_fetched < before.samples_fetched);
                assert_eq!(after.temporary_storage.extent.current, 0);
                provider.assert_closed();
                drop((expected, actual, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn completed_dry_progress_reuses_prefix_without_changing_window_results() {
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for expression in [
                "avg_over_time((sum by(instance)(counter_total))[4s:1s])",
                "quantile_over_time(0.5,(sum by(instance)(counter_total))[16s:1s])",
            ] {
                let engine = Engine::default();
                let control = Control::default();
                let provider = FixtureProvider::new(49, page, 8);
                let query = engine
                    .compile(
                        expression.as_bytes(),
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..Default::default()
                        },
                        &control,
                    )
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 16000,
                    end_ms: 48000,
                    step_ms: 1000,
                };
                let (baseline, before_hits, _) = crate::derived_progress::observe(true, || {
                    query.collect(&provider, request, &control)
                });
                let (resumed, hits, saves, checkpoint_bytes) =
                    crate::derived_progress::observe_with_reads(false, || {
                        query.collect(&provider, request, &control)
                    });
                assert_eq!(before_hits, 0);
                assert!(hits > 0 && saves > 0, "actual progress reuse: {expression}");
                let (expected, before) = baseline.unwrap();
                let (actual, after) = resumed.unwrap();
                same_float_results(&actual, &expected);
                assert_eq!(
                    after.statistics.samples.total_queryable_samples,
                    before.statistics.samples.total_queryable_samples
                );
                assert!(after.work < before.work, "work must fall: {expression}");
                assert!(
                    after.temporary_storage.read_bytes
                        <= before.temporary_storage.read_bytes + checkpoint_bytes,
                    "only measured checkpoint reads may add traffic: {expression}"
                );
                eprintln!(
                    "progress {expression}: work {} -> {}, backing reads {} -> {}, checkpoint reads {checkpoint_bytes}",
                    before.work,
                    after.work,
                    before.temporary_storage.read_bytes,
                    after.temporary_storage.read_bytes
                );
                drop((actual, expected, query));
                provider.assert_closed();
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn dry_progress_preserves_sparse_stale_windows_and_annotations() {
    fn sparse(index: usize) -> crate::value::Sample {
        let value = if matches!(index % 12, 0 | 1 | 2 | 7 | 8) {
            f64::from_bits(crate::value::STALE_NAN_BITS)
        } else {
            index as f64
        };
        crate::value::Sample {
            value: crate::value::SampleValue::Float(value),
            ..crate::test_storage::counter(index)
        }
    }
    for page in [1, 17, 1024] {
        for delayed in [false, true] {
            for expression in [
                "avg_over_time((sum by(instance)(counter_total))[2s:1s])",
                "quantile_over_time(2,(sum by(instance)(counter_total))[4s:1s])",
            ] {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(37, page, 4);
                for series in &mut provider.series {
                    series.source.sample = sparse;
                }
                let query = engine
                    .compile(
                        expression.as_bytes(),
                        EngineOptions {
                            delayed_name_removal: delayed,
                            ..Default::default()
                        },
                        &control,
                    )
                    .unwrap();
                let request = QueryRequest::Range {
                    start_ms: 4000,
                    end_ms: 36000,
                    step_ms: 1000,
                };
                let (baseline, _, _) = crate::derived_progress::observe(true, || {
                    query.collect(&provider, request, &control)
                });
                let (resumed, hits, saves) = crate::derived_progress::observe(false, || {
                    query.collect(&provider, request, &control)
                });
                assert!(
                    hits > 0 && saves > 0,
                    "actual sparse progress: {expression}"
                );
                let (expected, before) = baseline.unwrap();
                let (actual, after) = resumed.unwrap();
                same_float_results(&actual, &expected);
                if expression.starts_with("quantile") {
                    assert!(!actual.annotations.is_empty());
                }
                assert_eq!(
                    after.statistics.samples.total_queryable_samples,
                    before.statistics.samples.total_queryable_samples
                );
                drop((actual, expected, query));
                provider.assert_closed();
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
}

#[test]
fn root_timestamp_group_reuses_preparation_and_previous_final_selector_cells() {
    for delayed in [false, true] {
        let engine = Engine::default();
        let control = Control::default();
        let mut provider = FixtureProvider::new(33, 17, 32);
        for (index, series) in provider.series.iter_mut().enumerate() {
            series.labels[1].1 = (index / 2).to_string().into_bytes();
            series.labels[2].1 = (index % 2).to_string().into_bytes();
        }
        let query = engine
            .compile(
                b"max by(instance)(timestamp(counter_total))",
                EngineOptions {
                    delayed_name_removal: delayed,
                    ..Default::default()
                },
                &control,
            )
            .unwrap();
        let request = QueryRequest::Range {
            start_ms: 4000,
            end_ms: 32000,
            step_ms: 1000,
        };
        let (before, _) =
            computed_groups::observe(true, || query.collect(&provider, request, &control));
        let (after, hits) =
            computed_groups::observe(false, || query.collect(&provider, request, &control));
        let (expected, _) = before.unwrap();
        let (actual, report) = after.unwrap();
        same_float_results(&actual, &expected);
        eprintln!(
            "root-timestamp delayed={delayed} hits={hits:?} temporary={:?}",
            report.temporary_storage
        );
        assert!(
            hits[4] > 0,
            "preparation must actually reuse captured selector values"
        );
        assert!(
            hits[5] > 0,
            "later clocks must reuse the previous final selector cell"
        );
        drop((actual, expected, query));
        provider.assert_closed();
        engine.accounting.snapshot().assert_memory_released();
    }
}
