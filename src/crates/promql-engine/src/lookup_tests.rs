// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{result::OwnedValue, test_storage::FixtureProvider};

#[test]
fn registered_float_maps_preserve_vector_shapes_and_nan_membership() {
    let functions: Vec<_> = crate::frontend::functions::FUNCTIONS
        .iter()
        .filter(|function| {
            matches!(
                PointwiseFunction::parse(function.name),
                Some(PointwiseFunction::Float(..))
            )
        })
        .collect();
    assert_eq!(functions.len(), 35);
    let mut valid = 0;
    let mut invalid = 0;
    for function in functions {
        let invalid_arity = matches!(function.name, "clamp" | "clamp_min" | "clamp_max");
        if invalid_arity {
            invalid += 1;
        } else {
            valid += 1;
        }
        for page in [1, 17, 1024] {
            for delayed in [false, true] {
                let engine = Engine::default();
                let control = Control::default();
                let expression = format!(
                    "{0}(counter_total) == bool on(instance) {0}(counter_total)",
                    function.name
                );
                let compiled = engine.compile(
                    expression.as_bytes(),
                    EngineOptions {
                        delayed_name_removal: delayed,
                        ..EngineOptions::default()
                    },
                    &control,
                );
                if invalid_arity {
                    let error = compiled
                        .err()
                        .expect("clamp requires additional parameters");
                    assert!(matches!(&error, CompileError::Parse(_)));
                    drop(error);
                    engine.accounting.snapshot().assert_memory_released();
                    continue;
                }
                let query = compiled.unwrap();
                let provider = FixtureProvider::new(3, page, 4);
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
                assert_eq!(
                    result.series.len(),
                    4,
                    "{} page{page} delayed{delayed}",
                    function.name
                );
                assert!(report.annotations.is_empty());
                for (index, series) in result.series.iter().enumerate() {
                    assert_eq!(series.labels.0.len(), 1);
                    assert_eq!(&*series.labels.0[0].name, b"instance");
                    assert_eq!(&*series.labels.0[0].value, index.to_string().as_bytes());
                    assert_eq!(series.points.len(), 3);
                    for (tick, point) in series.points.iter().enumerate() {
                        assert_eq!(point.time_ms, (tick as i64 + 1) * 1000);
                        let OwnedValue::Float(value) = point.value else {
                            panic!("float comparison");
                        };
                        // These domain NaNs stay present and compare unequal to themselves.
                        let expected: f64 =
                            if tick > 0 && matches!(function.name, "asin" | "acos" | "atanh") {
                                0.0
                            } else {
                                1.0
                            };
                        assert_eq!(
                            value.to_bits(),
                            expected.to_bits(),
                            "{} tick{tick}",
                            function.name
                        );
                    }
                }
                assert_eq!(report.observations.output_samples, 12);
                assert_eq!(
                    report
                        .observations
                        .statistics
                        .samples
                        .total_queryable_samples,
                    24
                );
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
    assert_eq!((valid, invalid), (32, 3));
}

#[test]
fn completed_float_map_join_avoids_repeated_operand_reads() {
    fn point(index: usize) -> crate::value::Sample {
        crate::value::Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(-(index as f64 + 1.0)),
        }
    }
    let mut observations = Vec::new();
    for (function, left) in [
        ("abs", "abs(counter_total)"),
        ("ceil", "ceil(counter_total)"),
        ("round", "round(counter_total)"),
        ("year", "year(counter_total)"),
        ("round-param", "round(counter_total, .5)"),
        ("clamp-min", "clamp_min(counter_total, -4)"),
        ("clamp-max", "clamp_max(counter_total, 0)"),
        ("clamp", "clamp(counter_total, -4, 0)"),
    ] {
        for page_size in [1, 17, 1024] {
            for count in [32, 128] {
                let engine = Engine::default();
                let control = Control::default();
                let mut provider = FixtureProvider::new(3, page_size, count);
                for series in &mut provider.series {
                    series.source.sample = point;
                }
                let right = if function == "year" {
                    "year(counter_total)"
                } else {
                    "counter_total"
                };
                let expression = format!("{left} / on(instance) {right}");
                let query = engine
                    .compile(expression.as_bytes(), EngineOptions::default(), &control)
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
                assert_eq!(result.series.len(), count);
                assert!(report.annotations.is_empty());
                let mut instances: Vec<_> = (0..count).map(|index| index.to_string()).collect();
                instances.sort();
                for (instance, series) in instances.iter().zip(&*result.series) {
                    assert_eq!(series.labels.0.len(), 1);
                    assert_eq!(&*series.labels.0[0].name, b"instance");
                    assert_eq!(&*series.labels.0[0].value, instance.as_bytes());
                    let points: Vec<_> = series
                        .points
                        .iter()
                        .map(|point| {
                            let OwnedValue::Float(value) = point.value else {
                                panic!("float map pair");
                            };
                            (point.time_ms, value.to_bits())
                        })
                        .collect();
                    let expected: f64 = if function == "abs" { -1.0 } else { 1.0 };
                    assert_eq!(
                        points,
                        [
                            (1000, expected.to_bits()),
                            (2000, expected.to_bits()),
                            (3000, expected.to_bits())
                        ]
                    );
                }
                let observed = &report.observations;
                assert_eq!(observed.output_samples, 3 * count as u64);
                assert_eq!(
                    observed.statistics.samples.total_queryable_samples,
                    6 * count as u64
                );
                assert!(observed.statistics.samples.samples_read <= observed.samples_fetched);
                let counters = &provider.series[0].source.counters;
                assert_eq!(
                    observed.samples_fetched,
                    counters
                        .samples_returned
                        .load(std::sync::atomic::Ordering::Relaxed) as u64
                );
                assert_eq!(counters.peak.load(std::sync::atomic::Ordering::Relaxed), 1);
                println!(
                    "{{\"float_map_join\":\"{function}\",\"rows\":{count},\"page_size\":{page_size},\"fetched\":{},\"read\":{},\"queryable\":{},\"work\":{},\"workspace_peak\":{}}}",
                    observed.samples_fetched,
                    observed.statistics.samples.samples_read,
                    observed.statistics.samples.total_queryable_samples,
                    observed.work,
                    observed.memory[MemoryKind::Workspace as usize].peak
                );
                observations.push((observed.samples_fetched, observed.work));
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
    assert!(
        observations
            .chunks_exact(2)
            .all(|pair| pair[1].0 <= pair[0].0 * 6 && pair[1].1 <= pair[0].1 * 8),
        "completed float-map joins must avoid repeated full operand reads: {observations:?}"
    );
}

#[test]
fn completed_float_map_join_preserves_sparse_native_membership_and_wrapper_ordinals() {
    fn point<const MIXED: bool, const ID: usize>(index: usize) -> crate::value::Sample {
        use crate::value::{Sample, SampleValue};
        let stale = f64::from_bits(crate::value::STALE_NAN_BITS);
        let mixed = [
            [-1.0, -4.0, stale],
            [-10.0, 0.0, -9.0],
            [0.0; 3],
            [-10.0, -30.0, -5.0],
        ];
        let peer = [
            [2.0, 3.0, stale],
            [2.0, 2.0, 8.0],
            [3.0; 3],
            [20.0, 20.0, stale],
        ];
        Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: if MIXED && (ID == 2 || ID == 1 && index == 1) {
                SampleValue::FloatHistogram(crate::histogram::FloatHistogram::default())
            } else {
                SampleValue::Float(if MIXED {
                    mixed[ID][index]
                } else {
                    peer[ID][index]
                })
            },
        }
    }
    for (function, parameters) in [
        ("abs", ""),
        ("ceil", ""),
        ("round", ""),
        ("year", ""),
        ("round", ", .5"),
        ("clamp_min", ", 0"),
        ("clamp_max", ", -2"),
        ("clamp", ", -2, 2"),
    ] {
        for page in [1, 17, 1024] {
            for delayed in [false, true] {
                for route in ["left", "right", "both"] {
                    let mut provider = FixtureProvider::new(3, page, 8);
                    provider.filter_selectors = true;
                    let samples: [fn(usize) -> crate::value::Sample; 8] = [
                        point::<true, 2>,
                        point::<true, 0>,
                        point::<true, 3>,
                        point::<true, 1>,
                        point::<false, 3>,
                        point::<false, 1>,
                        point::<false, 0>,
                        point::<false, 2>,
                    ];
                    for (index, series) in provider.series.iter_mut().enumerate() {
                        series.labels = vec![
                            (
                                b"__name__".to_vec(),
                                if (index < 4) == (route != "right") {
                                    b"a"
                                } else {
                                    b"b"
                                }
                                .to_vec(),
                            ),
                            (
                                b"instance".to_vec(),
                                [2, 0, 3, 1, 3, 1, 0, 2][index].to_string().into_bytes(),
                            ),
                        ];
                        series.source.sample = samples[index];
                    }
                    let expression = match route {
                        "left" => format!("{function}(a{parameters}) - on(instance) b"),
                        "right" => format!("a - on(instance) {function}(b{parameters})"),
                        _ => format!(
                            "{function}(a{parameters}) - on(instance) {function}(b{parameters})"
                        ),
                    };
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
                    let expected_rows = match function {
                        "abs" => vec![
                            (0, vec![(1000, -1.0f64), (2000, 1.0)]),
                            (1, vec![(1000, 8.0), (3000, 1.0)]),
                            (3, vec![(1000, -10.0), (2000, 10.0)]),
                        ],
                        "year" if route == "both" => vec![
                            (0, vec![(1000, -1.0), (2000, -1.0)]),
                            (1, vec![(1000, -1.0), (3000, -1.0)]),
                            (3, vec![(1000, -1.0), (2000, -1.0)]),
                        ],
                        "year" => vec![
                            (0, vec![(1000, 1967.0), (2000, 1966.0)]),
                            (1, vec![(1000, 1967.0), (3000, 1961.0)]),
                            (3, vec![(1000, 1949.0), (2000, 1949.0)]),
                        ],
                        "clamp_min" => vec![
                            (0, vec![(1000, -2.0), (2000, -3.0)]),
                            (1, vec![(1000, -2.0), (3000, -8.0)]),
                            (3, vec![(1000, -20.0), (2000, -20.0)]),
                        ],
                        "clamp_max" if route == "both" => vec![
                            (0, vec![(1000, 0.0), (2000, -2.0)]),
                            (1, vec![(1000, -8.0), (3000, -7.0)]),
                            (3, vec![(1000, -8.0), (2000, -28.0)]),
                        ],
                        "clamp_max" => vec![
                            (0, vec![(1000, -4.0), (2000, -7.0)]),
                            (1, vec![(1000, -12.0), (3000, -17.0)]),
                            (3, vec![(1000, -30.0), (2000, -50.0)]),
                        ],
                        "clamp" if route == "both" => vec![
                            (0, vec![(1000, -3.0), (2000, -4.0)]),
                            (1, vec![(1000, -4.0), (3000, -4.0)]),
                            (3, vec![(1000, -4.0), (2000, -4.0)]),
                        ],
                        "clamp" => vec![
                            (0, vec![(1000, -3.0), (2000, -5.0)]),
                            (1, vec![(1000, -4.0), (3000, -10.0)]),
                            (3, vec![(1000, -22.0), (2000, -22.0)]),
                        ],
                        _ => vec![
                            (0, vec![(1000, -3.0), (2000, -7.0)]),
                            (1, vec![(1000, -12.0), (3000, -17.0)]),
                            (3, vec![(1000, -30.0), (2000, -50.0)]),
                        ],
                    };
                    for (series, (id, expected)) in result.series.iter().zip(expected_rows) {
                        assert_eq!(series.labels.0.len(), 1);
                        assert_eq!(&*series.labels.0[0].name, b"instance");
                        assert_eq!(&*series.labels.0[0].value, id.to_string().as_bytes());
                        let points: Vec<_> = series
                            .points
                            .iter()
                            .map(|p| {
                                let OwnedValue::Float(value) = p.value else {
                                    panic!("float map join");
                                };
                                (p.time_ms, value.to_bits())
                            })
                            .collect();
                        let expected: Vec<_> = expected
                            .into_iter()
                            .map(|(t, v)| (t, (if route == "right" { -v } else { v }).to_bits()))
                            .collect();
                        assert_eq!(
                            points, expected,
                            "{function}{parameters} {route} page{page} delayed{delayed}"
                        );
                    }
                    assert_eq!(report.observations.output_samples, 6);
                    assert_eq!(
                        provider.series[0]
                            .source
                            .counters
                            .peak
                            .load(std::sync::atomic::Ordering::Relaxed),
                        1
                    );
                    provider.assert_closed();
                    drop((report, query));
                    engine.accounting.snapshot().assert_memory_released();
                }
            }
        }
    }
}

#[test]
fn completed_sort_emission_avoids_repeated_sorted_operands() {
    let mut observations = Vec::new();
    for page in [1, 17, 1024] {
        for count in [32, 128] {
            let engine = Engine::default();
            let control = Control::default();
            let provider = FixtureProvider::new(3, page, count);
            let mut options = EngineOptions::default();
            options.parser.experimental_functions = true;
            let query = engine
                .compile(
                    b"sort_by_label(counter_total,\"instance\")",
                    options,
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
            assert_eq!(result.series.len(), count);
            assert_eq!(report.annotations.len(), 1);
            let mut expected: Vec<_> = (0..count).map(|index| index.to_string()).collect();
            expected.sort();
            for (series, instance) in result.series.iter().zip(&expected) {
                assert_eq!(series.labels.0.len(), 3);
                for (label, (name, value)) in series.labels.0.iter().zip([
                    (b"__name__".as_slice(), b"counter_total".as_slice()),
                    (b"instance", instance.as_bytes()),
                    (b"job", b"worker"),
                ]) {
                    assert_eq!(&*label.name, name);
                    assert_eq!(&*label.value, value);
                }
                assert_eq!(series.points.len(), 3);
                for (index, point) in series.points.iter().enumerate() {
                    assert_eq!(point.time_ms, (index as i64 + 1) * 1000);
                    let OwnedValue::Float(value) = point.value else {
                        panic!("float sorted operand");
                    };
                    assert_eq!(value.to_bits(), (index as f64 + 1.0).to_bits());
                }
            }
            let observed = &report.observations;
            assert_eq!(observed.output_samples, 3 * count as u64);
            assert_eq!(
                observed.statistics.samples.total_queryable_samples,
                3 * count as u64
            );
            assert!(observed.statistics.samples.samples_read <= observed.samples_fetched);
            assert_eq!(
                observed.samples_fetched,
                provider.series[0]
                    .source
                    .counters
                    .samples_returned
                    .load(std::sync::atomic::Ordering::Relaxed) as u64
            );
            assert_eq!(
                provider.series[0]
                    .source
                    .counters
                    .peak
                    .load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            println!(
                "{{\"sort_emission_rows\":{count},\"page_size\":{page},\"fetched\":{},\"read\":{},\"queryable\":{},\"work\":{},\"workspace_peak\":{}}}",
                observed.samples_fetched,
                observed.statistics.samples.samples_read,
                observed.statistics.samples.total_queryable_samples,
                observed.work,
                observed.memory[MemoryKind::Workspace as usize].peak
            );
            observations.push((observed.samples_fetched, observed.work));
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
    assert!(
        observations
            .chunks_exact(2)
            .all(|pair| pair[1].0 <= pair[0].0 * 6 && pair[1].1 <= pair[0].1 * 8),
        "completed sorted delivery must avoid repeated full operand reads: {observations:?}"
    );
}

#[test]
fn quantile_group_keys_do_not_retain_discarded_label_capacity() {
    let mut retained = Vec::new();
    for count in [8, 17, 65] {
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(
                b"quantile by(group,job)(0.5,m)",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let account = engine.accounting.query();
        let mut labels = crate::result::OwnedLabels::new(&account, MemoryKind::Workspace);
        for (name, value) in [
            (b"__name__".as_slice(), b"m".as_slice()),
            (b"group", b"g"),
            (b"job", b"worker"),
        ] {
            labels.push(name, value, &account, MemoryKind::Workspace);
        }
        for position in 3..count {
            labels.push(
                format!("tag_{position:03}").as_bytes(),
                b"discarded metadata",
                &account,
                MemoryKind::Workspace,
            );
        }
        let hash = crate::label_hash::full(&labels, &control, &account).unwrap();
        let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
        rows.push(crate::evaluator_roster::Descriptor {
            labels,
            ordinal: 0,
            hash,
            drop_name: false,
            first_order: 0,
            sources: 0..1,
            kinds: 1,
            heads: Default::default(),
        });
        let mut parameters = crate::topk::Parameters::new(true);
        parameters.push(0.5, &account);
        let mut plan = crate::quantile_aggregate::Plan::new(parameters, &account);
        let mut expected = crate::result::OwnedLabels::new(&account, MemoryKind::Workspace);
        for (name, value) in [(b"group".as_slice(), b"g".as_slice()), (b"job", b"worker")] {
            expected.push(name, value, &account, MemoryKind::Workspace);
        }
        let expected_hash = crate::label_hash::grouping(&expected, &control, &account).unwrap();
        let groups = plan
            .prepare_rows(
                rows,
                &query.expression,
                query.expression.root,
                &control,
                &account,
            )
            .unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].labels.compare(&expected),
            std::cmp::Ordering::Equal
        );
        assert_eq!(groups[0].hash, expected_hash);
        assert!(groups[0].whole_drop_name.is_none());
        let before_drop = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        drop(groups);
        let bytes = before_drop - account.snapshot().memory[MemoryKind::Workspace as usize].current;
        println!("{{\"grouping_input_labels\":{count},\"retained_group_bytes\":{bytes}}}");
        retained.push(bytes);
        drop((plan, query, expected));
        account.snapshot().assert_memory_released();
        drop(account);
        engine.accounting.snapshot().assert_memory_released();
    }
    assert!(
        retained.iter().all(|bytes| *bytes == retained[0]),
        "two kept grouping labels must not retain input-sized capacity: {retained:?}"
    );
}

fn nested_set_observations(count: usize, page_size: usize, padding: usize) -> QueryObservations {
    nested_set_case(count, page_size, padding, false)
}

fn nested_set_case(
    count: usize,
    page_size: usize,
    padding: usize,
    sparse: bool,
) -> QueryObservations {
    fn point(index: usize) -> crate::value::Sample {
        crate::value::Sample {
            time_ms: (index as i64 + 2) * 1000,
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(1.0),
        }
    }
    let engine = Engine::default();
    let control = Control::default();
    let mut provider = FixtureProvider::new(3, page_size, count);
    for (position, series) in provider.series.iter_mut().enumerate() {
        series.source.sample = point;
        if sparse && position != 0 && position != count - 1 {
            series.source.count = 1;
        }
        if padding != 0 {
            series
                .labels
                .push((b"padding".to_vec(), vec![b'x'; padding]));
        }
    }
    let query = engine
                .compile(
                    b"sum(label_replace(counter_total,\"branch\",\"a\",\"instance\",\".*\") or label_replace(counter_total,\"branch\",\"b\",\"instance\",\".*\"))",
                    EngineOptions {
                        lookback_ms: if sparse { 100 } else { 300000 },
                        ..EngineOptions::default()
                    },
                    &control,
                )
                .unwrap();
    let report = query.collect_reported(
        &provider,
        QueryRequest::Range {
            start_ms: 2000,
            end_ms: 4000,
            step_ms: 1000,
        },
        &control,
    );
    let result = report.result.as_ref().unwrap();
    assert!(report.annotations.is_empty());
    assert_eq!(result.series.len(), 1);
    assert!(result.series[0].labels.0.is_empty());
    let points: Vec<_> = result.series[0]
        .points
        .iter()
        .map(|point| {
            let OwnedValue::Float(value) = point.value else {
                panic!("float set sum");
            };
            (point.time_ms, value.to_bits())
        })
        .collect();
    assert_eq!(
        points,
        [2000, 3000, 4000].map(|time| {
            let value = if sparse && time != 2000 {
                4.0
            } else {
                2.0 * count as f64
            };
            (time, value.to_bits())
        })
    );
    let observed = &report.observations;
    assert_eq!(observed.output_samples, 3);
    assert_eq!(
        observed.statistics.samples.total_queryable_samples,
        if sparse {
            2 * count as u64 + 8
        } else {
            6 * count as u64
        }
    );
    assert_eq!(
        observed.samples_fetched,
        provider.series[0]
            .source
            .counters
            .samples_returned
            .load(std::sync::atomic::Ordering::Relaxed) as u64
    );
    assert_eq!(
        provider.series[0]
            .source
            .counters
            .peak
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
    println!(
        "{{\"nested_set_rows\":{count},\"page_size\":{page_size},\"label_bytes\":{padding},\"sparse\":{sparse},\"fetched\":{},\"read\":{},\"queryable\":{},\"work\":{},\"workspace_peak\":{}}}",
        observed.samples_fetched,
        observed.statistics.samples.samples_read,
        observed.statistics.samples.total_queryable_samples,
        observed.work,
        observed.memory[MemoryKind::Workspace as usize].peak
    );
    let observations = observed.clone();
    provider.assert_closed();
    drop((report, query));
    engine.accounting.snapshot().assert_memory_released();
    observations
}

#[test]
fn nested_set_sparse_members_preserve_complete_paged_results() {
    for page_size in [1, 17, 1024] {
        for count in [128, 512] {
            nested_set_case(count, page_size, 0, true);
        }
    }
}

#[test]
fn nested_set_prefix_reuses_successful_operands() {
    let mut reads = Vec::new();
    for page_size in [1, 17, 1024] {
        for count in [16, 32, 64] {
            let observations = nested_set_observations(count, page_size, 0);
            reads.push((count, page_size, observations.samples_fetched));
        }
    }
    assert!(
        reads
            .iter()
            .all(|(count, _, fetched)| *fetched <= 1000 * *count as u64),
        "completed set prefixes must avoid repeated operand acquisition: {reads:?}"
    );
}

#[test]
fn nested_set_prefix_reuses_canonical_label_hashes() {
    let mut work = Vec::new();
    for page_size in [1, 17, 1024] {
        let observations =
            [16, 32, 64].map(|count| nested_set_observations(count, page_size, 65536));
        work.push((page_size, observations.map(|observed| observed.work)));
    }
    assert!(
        work.iter().all(|(_, work)| work[2] <= 3 * work[1]),
        "doubling large-label set rows must use at most three times the work: {work:?}"
    );
}

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

fn histogram_lookup_work(count: usize, page_size: usize, range: bool) -> u64 {
    let ticks = if range { 8 } else { 1 };
    let engine = Engine::default();
    let control = Control::default();
    let mut provider = FixtureProvider::new(if range { 8 } else { 2 }, page_size, 2 * count);
    for (index, series) in provider.series.iter_mut().enumerate() {
        series.labels[0].1 = b"request_duration_bucket".to_vec();
        series.labels[1].1 = format!("{:08}", index / 2).into_bytes();
        series.labels.push((
            b"le".to_vec(),
            if index % 2 == 0 {
                b"1".to_vec()
            } else {
                b"+Inf".to_vec()
            },
        ));
    }
    let query = engine
        .compile(
            b"histogram_quantile(0.5,request_duration_bucket)",
            EngineOptions::default(),
            &control,
        )
        .unwrap();
    let report = query.collect_reported(
        &provider,
        if range {
            QueryRequest::Range {
                start_ms: 1000,
                end_ms: 8000,
                step_ms: 1000,
            }
        } else {
            QueryRequest::Instant { time_ms: 2000 }
        },
        &control,
    );
    let result = report.result.as_ref().unwrap();
    assert_eq!(result.series.len(), count);
    assert!(report.annotations.is_empty());
    for (index, series) in result.series.iter().enumerate() {
        assert_eq!(series.labels.0.len(), 2);
        assert_eq!(&*series.labels.0[0].name, b"instance");
        assert_eq!(&*series.labels.0[0].value, format!("{index:08}").as_bytes());
        assert_eq!(&*series.labels.0[1].name, b"job");
        assert_eq!(&*series.labels.0[1].value, b"worker");
        assert_eq!(series.points.len(), ticks);
        for (position, point) in series.points.iter().enumerate() {
            assert_eq!(
                point.time_ms,
                if range {
                    1000 * (position as i64 + 1)
                } else {
                    2000
                }
            );
            let OwnedValue::Float(value) = point.value else {
                panic!("classic quantile must emit floats");
            };
            assert_eq!(value.to_bits(), 0.5f64.to_bits());
        }
    }
    assert_eq!(report.observations.output_samples, (count * ticks) as u64);
    assert_eq!(
        report
            .observations
            .statistics
            .samples
            .total_queryable_samples,
        (2 * count * ticks) as u64
    );
    let expected_read = if range {
        match count {
            16 => 24448,
            32 => 85760,
            64 => 318976,
            _ => unreachable!("declared range statistics cohort"),
        }
    } else {
        (14 * count) as u64
    };
    assert_eq!(
        report.observations.statistics.samples.samples_read,
        expected_read
    );
    assert_eq!(
        report.observations.samples_fetched,
        provider.series[0]
            .source
            .counters
            .samples_returned
            .load(std::sync::atomic::Ordering::Relaxed) as u64
    );
    let work = report.observations.work;
    println!(
        "{{\"histogram_groups\":{count},\"page_size\":{page_size},\"ticks\":{ticks},\"work\":{work},\"fetched\":{},\"workspace_peak\":{},\"queryable\":{},\"samples_read\":{}}}",
        report.observations.samples_fetched,
        report.observations.memory[MemoryKind::Workspace as usize].peak,
        report
            .observations
            .statistics
            .samples
            .total_queryable_samples,
        report.observations.statistics.samples.samples_read
    );
    provider.assert_closed();
    drop((report, query));
    engine.accounting.snapshot().assert_memory_released();
    work
}

#[test]
fn histogram_constructor_lookup_avoids_scanning_every_prior_key() {
    let mut measured = Vec::new();
    for page in [1, 17, 1024] {
        for count in [128, 512] {
            measured.push((count, page, histogram_lookup_work(count, page, false)));
        }
    }
    for (count, page, work) in measured {
        assert!(
            work <= (20_000 * count) as u64,
            "{count} histogram groups, page {page} used work {work}"
        );
    }
}

#[test]
fn histogram_active_distribution_reduces_complete_instant_and_range_work() {
    let mut measured = Vec::new();
    for page in [1, 17, 1024] {
        for (count, range, bound) in [
            (128, false, 835000),
            (512, false, 5000000),
            (16, true, 1950000),
            (32, true, 8000000),
            (64, true, 35000000),
        ] {
            measured.push((
                count,
                page,
                range,
                bound,
                histogram_lookup_work(count, page, range),
            ));
        }
    }
    for (count, page, range, bound, work) in measured {
        assert!(
            work <= bound,
            "{count} histogram groups, page{page}, range={range} used work {work}>{bound}"
        );
    }
}

#[test]
fn binary_matching_lookup_work_avoids_all_pairs() {
    let work = cardinality_work(b"counter_total / on (instance) counter_total");
    assert!(work[1] <= work[0] * 6, "4x rows used work {work:?}");
}

#[test]
fn range_preparation_uniqueness_avoids_scanning_every_prior_hash() {
    let mut measured = Vec::new();
    for page_size in [1, 17, 1024] {
        for count in [128, 512] {
            let engine = Engine::default();
            let control = Control::default();
            let mut provider = FixtureProvider::new(2, page_size, count);
            for (index, series) in provider.series.iter_mut().enumerate() {
                series.labels[1].1 = format!("{index:08}").into_bytes();
            }
            let query = engine
                .compile(
                    b"avg_over_time(counter_total[5m])",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 2000 },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), count);
            assert!(report.annotations.is_empty());
            for (index, series) in result.series.iter().enumerate() {
                assert_eq!(series.labels.0.len(), 2);
                assert_eq!(&*series.labels.0[0].name, b"instance");
                assert_eq!(&*series.labels.0[0].value, format!("{index:08}").as_bytes());
                assert_eq!(&*series.labels.0[1].name, b"job");
                assert_eq!(&*series.labels.0[1].value, b"worker");
                assert_eq!(series.points.len(), 1);
                assert_eq!(series.points[0].time_ms, 2000);
                let OwnedValue::Float(value) = series.points[0].value else {
                    panic!("range reduction must emit floats");
                };
                assert_eq!(value.to_bits(), 1.5f64.to_bits());
            }
            assert_eq!(report.observations.output_samples, count as u64);
            assert_eq!(
                report
                    .observations
                    .statistics
                    .samples
                    .total_queryable_samples,
                (2 * count) as u64
            );
            let work = report.observations.work;
            println!(
                "{{\"range_unique_series\":{count},\"page_size\":{page_size},\"work\":{work},\"fetched\":{},\"workspace_peak\":{}}}",
                report.observations.samples_fetched,
                report.observations.memory[MemoryKind::Workspace as usize].peak
            );
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
            measured.push((count, page_size, work));
        }
    }
    for (count, page, work) in measured {
        assert!(
            work <= (900 * count) as u64,
            "{count} series, page {page} used work {work}"
        );
    }
}

#[test]
fn raw_float_average_reuses_validated_acquisition_without_extra_reads() {
    let steps = 8;
    let mut measured = Vec::new();
    for page_size in [1, 17, 1024] {
        for count in [128, 512] {
            let engine = Engine::default();
            let control = Control::default();
            let mut provider = FixtureProvider::new(2, page_size, count);
            for (index, series) in provider.series.iter_mut().enumerate() {
                series.labels[1].1 = format!("{index:08}").into_bytes();
            }
            let query = engine
                .compile(
                    b"avg_over_time(counter_total[5m])",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 2000,
                    end_ms: 9000,
                    step_ms: 1000,
                },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), count);
            assert!(report.annotations.is_empty());
            for (index, series) in result.series.iter().enumerate() {
                assert_eq!(series.labels.0.len(), 2);
                assert_eq!(&*series.labels.0[0].name, b"instance");
                assert_eq!(&*series.labels.0[0].value, format!("{index:08}").as_bytes());
                assert_eq!(&*series.labels.0[1].name, b"job");
                assert_eq!(&*series.labels.0[1].value, b"worker");
                assert_eq!(series.points.len(), steps);
                for (step, point) in series.points.iter().enumerate() {
                    assert_eq!(point.time_ms, 2000 + step as i64 * 1000);
                    let OwnedValue::Float(value) = point.value else {
                        panic!("numeric range must emit floats");
                    };
                    assert_eq!(value.to_bits(), 1.5f64.to_bits());
                }
            }
            assert_eq!(report.observations.output_samples, (count * steps) as u64);
            assert_eq!(
                report
                    .observations
                    .statistics
                    .samples
                    .total_queryable_samples,
                (2 * count * steps) as u64
            );
            assert_eq!(
                report.observations.statistics.samples.samples_read,
                (4 * count * steps) as u64
            );
            let work = report.observations.work;
            let fetched = report.observations.samples_fetched;
            println!(
                "{{\"avg_reuse_series\":{count},\"page_size\":{page_size},\"work\":{work},\"fetched\":{},\"workspace_peak\":{}}}",
                report.observations.samples_fetched,
                report.observations.memory[MemoryKind::Workspace as usize].peak
            );
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
            measured.push((count, page_size, fetched));
        }
    }
    for (count, page, fetched) in measured {
        assert!(
            fetched <= (8 * count * steps) as u64,
            "{count} series, page {page} fetched sample entries {fetched}"
        );
    }
}

#[test]
fn direct_numeric_range_preparation_avoids_repeated_summary_scans() {
    let steps = 8;
    let mut measured = Vec::new();
    for page_size in [1, 17, 1024] {
        for count in [128, 512] {
            let engine = Engine::default();
            let control = Control::default();
            let mut provider = FixtureProvider::new(2, page_size, count);
            for (index, series) in provider.series.iter_mut().enumerate() {
                series.labels[1].1 = format!("{index:08}").into_bytes();
            }
            let query = engine
                .compile(
                    b"avg_over_time(counter_total[5m])",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 2000,
                    end_ms: 9000,
                    step_ms: 1000,
                },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), count);
            assert!(report.annotations.is_empty());
            for (index, series) in result.series.iter().enumerate() {
                assert_eq!(series.labels.0.len(), 2);
                assert_eq!(&*series.labels.0[0].name, b"instance");
                assert_eq!(&*series.labels.0[0].value, format!("{index:08}").as_bytes());
                assert_eq!(&*series.labels.0[1].name, b"job");
                assert_eq!(&*series.labels.0[1].value, b"worker");
                assert_eq!(series.points.len(), steps);
                for (step, point) in series.points.iter().enumerate() {
                    assert_eq!(point.time_ms, 2000 + step as i64 * 1000);
                    let OwnedValue::Float(value) = point.value else {
                        panic!("numeric range must emit floats");
                    };
                    assert_eq!(value.to_bits(), 1.5f64.to_bits());
                }
            }
            assert_eq!(report.observations.output_samples, (count * steps) as u64);
            assert_eq!(
                report
                    .observations
                    .statistics
                    .samples
                    .total_queryable_samples,
                (2 * count * steps) as u64
            );
            let work = report.observations.work;
            println!(
                "{{\"numeric_range_series\":{count},\"page_size\":{page_size},\"work\":{work},\"fetched\":{},\"workspace_peak\":{}}}",
                report.observations.samples_fetched,
                report.observations.memory[MemoryKind::Workspace as usize].peak
            );
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
            measured.push((count, page_size, work));
        }
    }
    for (count, page, work) in measured {
        assert!(
            work <= (850 * count * steps) as u64,
            "{count} series, page {page} used work {work}"
        );
    }
}

#[test]
fn label_heavy_join_discards_output_with_exact_series_and_statistics() {
    struct Discard {
        count: usize,
        labels: Vec<Vec<u8>>,
        next: usize,
        committed: bool,
        aborted: bool,
    }
    impl OutputSink for Discard {
        fn begin(&mut self, header: OutputHeader) -> Result<(), AccountedVec<u8>> {
            assert_eq!(
                (header.start_ms, header.end_ms, header.step_ms),
                (1000, 2000, 1000)
            );
            Ok(())
        }
        fn chunk(&mut self, chunk: OutputChunk<'_>) -> Result<(), AccountedVec<u8>> {
            let OutputChunk::Series { labels, points, .. } = chunk else {
                panic!("series output");
            };
            assert_eq!(labels.len(), 1);
            assert_eq!(labels[0].name, b"instance");
            assert_eq!(labels[0].value, self.labels[self.next % self.count]);
            assert_eq!(points.len(), 1);
            assert_eq!(
                points[0].time_ms,
                1000 + (self.next / self.count) as i64 * 1000
            );
            let crate::value::SampleValueRef::Float(value) = points[0].value else {
                panic!("float join");
            };
            assert_eq!(value.to_bits(), 1.0f64.to_bits());
            self.next += 1;
            Ok(())
        }
        fn commit(&mut self, observations: &QueryObservations) -> Result<(), AccountedVec<u8>> {
            assert_eq!(self.next, 2 * self.count);
            assert_eq!(observations.output_samples, (2 * self.count) as u64);
            assert_eq!(
                observations.statistics.samples.total_queryable_samples,
                (4 * self.count) as u64
            );
            self.committed = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.aborted = true;
        }
    }
    for page_size in [1, 17, 1024] {
        for count in [32, 128] {
            let mut provider = FixtureProvider::new(1, page_size, count);
            for series in &mut provider.series {
                for tag in 0..14 {
                    series
                        .labels
                        .push((format!("tag_{tag:02}").into_bytes(), b"metadata".to_vec()));
                }
            }
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(
                    b"counter_total / on(instance) counter_total",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let mut labels: Vec<_> = (0..count)
                .map(|index| index.to_string().into_bytes())
                .collect();
            labels.sort();
            let mut sink = Discard {
                count,
                labels,
                next: 0,
                committed: false,
                aborted: false,
            };
            let observations = query
                .execute(
                    &provider,
                    QueryRequest::Range {
                        start_ms: 1000,
                        end_ms: 2000,
                        step_ms: 1000,
                    },
                    &control,
                    &mut sink,
                )
                .unwrap();
            assert!(sink.committed && !sink.aborted);
            println!(
                "{{\"label_join_discard_rows\":{count},\"page_size\":{page_size},\"workspace_peak\":{},\"work\":{},\"fetched\":{},\"queryable\":{}}}",
                observations.memory[MemoryKind::Workspace as usize].peak,
                observations.work,
                observations.samples_fetched,
                observations.statistics.samples.total_queryable_samples
            );
            provider.assert_closed();
            drop(query);
            engine.accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn projected_derived_empty_name_errors_before_on_can_remove_it() {
    for page_size in [1, 17, 1024] {
        for delayed in [false, true] {
            let mut provider = FixtureProvider::new(2, page_size, 2);
            for (index, series) in provider.series.iter_mut().enumerate() {
                series.labels[1].1 = b"same".to_vec();
                series.labels.push((
                    b"le".to_vec(),
                    if index == 0 {
                        b"1".to_vec()
                    } else {
                        b"+Inf".to_vec()
                    },
                ));
            }
            let engine = Engine::default();
            let control = Control::default();
            let mut options = EngineOptions {
                delayed_name_removal: delayed,
                ..EngineOptions::default()
            };
            options.parser.experimental_functions = true;
            let query = engine.compile(
                b"histogram_quantiles(counter_total,\"\",0.5) / on(instance) histogram_quantiles(counter_total,\"\",0.5)",
                options, &control,
            ).unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 2000 },
                &control,
            );
            let Err(ExecutionError::Evaluation(crate::evaluator::EvaluationError::WrappedRuntime(
                message,
            ))) = &report.result
            else {
                panic!("derived empty name must retain its reserved-removal failure");
            };
            assert_eq!(
                &**message,
                b"unexpected error: runtime error: index out of range [0] with length 0"
            );
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn instant_root_numeric_fold_avoids_a_second_complete_quantile_scan() {
    let mut reads = Vec::new();
    for page_size in [1, 17, 1024] {
        let engine = Engine::default();
        let control = Control::default();
        let provider = FixtureProvider::new(4096, page_size, 1);
        let query = engine
            .compile(
                b"quantile_over_time(.5,counter_total[4096s])",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let report = query.collect_reported(
            &provider,
            QueryRequest::Instant { time_ms: 4_096_000 },
            &control,
        );
        let result = report.result.as_ref().unwrap();
        assert_eq!(result.series.len(), 1);
        assert!(report.annotations.is_empty());
        let series = &result.series[0];
        assert_eq!(series.labels.0.len(), 2);
        assert_eq!(&*series.labels.0[0].name, b"instance");
        assert_eq!(&*series.labels.0[0].value, b"0");
        assert_eq!(&*series.labels.0[1].name, b"job");
        assert_eq!(&*series.labels.0[1].value, b"worker");
        assert_eq!(series.points.len(), 1);
        assert_eq!(series.points[0].time_ms, 4_096_000);
        let OwnedValue::Float(value) = series.points[0].value else {
            panic!("float quantile");
        };
        assert_eq!(value.to_bits(), 2048.5f64.to_bits());
        let observed = &report.observations;
        assert_eq!(observed.output_samples, 1);
        assert_eq!(observed.statistics.samples.total_queryable_samples, 4096);
        println!(
            "{{\"instant_quantile_points\":4096,\"page_size\":{page_size},\"fetched\":{},\"work\":{},\"workspace_peak\":{},\"queryable\":{},\"samples_read\":{}}}",
            observed.samples_fetched,
            observed.work,
            observed.memory[MemoryKind::Workspace as usize].peak,
            observed.statistics.samples.total_queryable_samples,
            observed.statistics.samples.samples_read,
        );
        reads.push((page_size, observed.samples_fetched));
        provider.assert_closed();
        drop((report, query));
        engine.accounting.snapshot().assert_memory_released();
    }
    assert!(
        reads.iter().all(|(_, fetched)| *fetched < 50 * 4096),
        "instant quantile must avoid a duplicate full fold: {reads:?}",
    );
}

#[test]
fn instant_root_numeric_fold_reuses_absent_physical_series() {
    fn histogram(index: usize) -> crate::value::Sample {
        let mut sample = crate::test_storage::counter(index);
        sample.value = crate::value::SampleValue::FloatHistogram(Default::default());
        sample
    }
    for page_size in [1, 17, 1024] {
        for native_first in [false, true] {
            let mut provider = FixtureProvider::new(4096, page_size, 2);
            let native = usize::from(!native_first);
            provider.series[native].source.sample = histogram;
            provider.series[native].source.counters =
                std::sync::Arc::new(crate::test_storage::CursorCounts::default());
            let engine = Engine::default();
            let control = Control::default();
            let query = engine
                .compile(
                    b"quantile_over_time(.5,counter_total[4096s])",
                    EngineOptions::default(),
                    &control,
                )
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 4_096_000 },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), 1);
            assert!(report.annotations.is_empty());
            let series = &result.series[0];
            assert_eq!(series.labels.0.len(), 2);
            assert_eq!(&*series.labels.0[0].name, b"instance");
            assert_eq!(
                &*series.labels.0[0].value,
                (1 - native).to_string().as_bytes()
            );
            assert_eq!(&*series.labels.0[1].name, b"job");
            assert_eq!(&*series.labels.0[1].value, b"worker");
            assert_eq!(series.points.len(), 1);
            assert_eq!(series.points[0].time_ms, 4_096_000);
            let OwnedValue::Float(value) = series.points[0].value else {
                panic!("float quantile");
            };
            assert_eq!(value.to_bits(), 2048.5f64.to_bits());
            let absent_reads = provider.series[native]
                .source
                .counters
                .samples_returned
                .load(std::sync::atomic::Ordering::Relaxed);
            println!(
                "{{\"absent_numeric_rows\":1,\"native_first\":{native_first},\"page_size\":{page_size},\"absent_fetched\":{absent_reads},\"fetched\":{}}}",
                report.observations.samples_fetched
            );
            assert!(
                absent_reads < 4 * 4096,
                "absent physical series repeated its fold: page={page_size}, reads={absent_reads}"
            );
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
        let mut provider = FixtureProvider::new(4096, page_size, 1);
        provider.series[0].source.sample = histogram;
        let engine = Engine::default();
        let control = Control::default();
        let query = engine
            .compile(
                b"quantile_over_time(.5,counter_total[4096s])",
                EngineOptions::default(),
                &control,
            )
            .unwrap();
        let report = query.collect_reported(
            &provider,
            QueryRequest::Instant { time_ms: 4_096_000 },
            &control,
        );
        assert!(report.result.as_ref().unwrap().series.is_empty());
        assert!(report.annotations.is_empty());
        assert_eq!(report.observations.output_samples, 0);
        provider.assert_closed();
        drop((report, query));
        engine.accounting.snapshot().assert_memory_released();
    }
}

#[test]
fn instant_root_numeric_fold_keeps_raw_memory_independent_of_history() {
    for page_size in [1, 17, 1024] {
        let mut peaks = Vec::new();
        for history in [2048, 8192] {
            let provider = FixtureProvider::new(history, page_size, 1);
            let engine = Engine::default();
            let control = Control::default();
            let expression = format!("quantile_over_time(.5,counter_total[{history}s])");
            let query = engine
                .compile(expression.as_bytes(), EngineOptions::default(), &control)
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant {
                    time_ms: history as i64 * 1000,
                },
                &control,
            );
            let result = report.result.as_ref().unwrap();
            assert_eq!(result.series.len(), 1);
            assert!(report.annotations.is_empty());
            assert_eq!(result.series[0].points.len(), 1);
            assert_eq!(result.series[0].points[0].time_ms, history as i64 * 1000);
            let OwnedValue::Float(value) = result.series[0].points[0].value else {
                panic!("float quantile");
            };
            assert_eq!(value.to_bits(), (history as f64 / 2.0 + 0.5).to_bits());
            assert_eq!(
                provider.series[0]
                    .source
                    .counters
                    .peak
                    .load(std::sync::atomic::Ordering::Relaxed),
                1
            );
            peaks.push([
                report.observations.memory[MemoryKind::Workspace as usize].peak,
                report.observations.memory[MemoryKind::ProviderHeld as usize].peak,
                report.observations.memory[MemoryKind::BorrowedView as usize].peak,
            ]);
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
        assert_eq!(
            peaks[0], peaks[1],
            "history changes page={page_size}: {peaks:?}"
        );
    }
}

fn window_join_discovery_cases(expression: &[u8], options: EngineOptions) -> Vec<u64> {
    let count = 64;
    let mut work = Vec::new();
    for page_size in [1, 17, 1024] {
        for reversed in [false, true] {
            let mut provider = FixtureProvider::new(2, page_size, 2 * count);
            provider.filter_selectors = true;
            for (position, series) in provider.series.iter_mut().enumerate() {
                let index = if position < count || !reversed {
                    position % count
                } else {
                    2 * count - 1 - position
                };
                series.labels = vec![
                    (
                        b"__name__".to_vec(),
                        if position < count {
                            b"m_total"
                        } else {
                            b"n_total"
                        }
                        .to_vec(),
                    ),
                    (b"instance".to_vec(), index.to_string().into_bytes()),
                ];
            }
            let engine = Engine::default();
            let control = Control::default();
            let query = engine.compile(expression, options, &control).unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Range {
                    start_ms: 2000,
                    end_ms: 3000,
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
                            panic!("float rate join");
                        };
                        (point.time_ms, value.to_bits())
                    })
                    .collect();
                assert_eq!(points, [(2000, 1.0f64.to_bits()), (3000, 1.0f64.to_bits())]);
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
                "{{\"rate_join_rows\":{count},\"page_size\":{page_size},\"reversed\":{reversed},\"query\":{:?},\"work\":{},\"fetched\":{},\"workspace_peak\":{},\"queryable\":{}}}",
                std::str::from_utf8(expression).unwrap(),
                observed.work,
                observed.samples_fetched,
                observed.memory[MemoryKind::Workspace as usize].peak,
                observed.statistics.samples.total_queryable_samples,
            );
            work.push(observed.work);
            provider.assert_closed();
            drop((report, query));
            engine.accounting.snapshot().assert_memory_released();
        }
    }
    work
}

#[test]
fn direct_rate_join_avoids_repeating_full_child_discovery() {
    let work = window_join_discovery_cases(
        b"rate(m_total[5m]) / on(instance) rate(n_total[5m])",
        EngineOptions::default(),
    );
    assert!(
        work.iter().all(|value| *value < 1_650_000),
        "direct rate joins must avoid repeated whole-child discovery: {work:?}"
    );
}

#[test]
fn mapped_rate_join_avoids_repeating_full_child_discovery() {
    let mut work = Vec::new();
    for expression in [
        "clamp_min(rate(m_total[5m]),0) / on(instance) rate(n_total[5m])",
        "rate(m_total[5m]) / on(instance) abs(rate(n_total[5m]))",
        "abs(clamp_min(rate(m_total[5m]),-time())) / on(instance) abs(rate(n_total[5m]))",
        "(100 * +rate(m_total[5m])) / on(instance) (rate(n_total[5m]) * 100)",
    ] {
        work.extend(window_join_discovery_cases(
            expression.as_bytes(),
            EngineOptions::default(),
        ));
    }
    assert!(
        work.iter().all(|value| *value < 1_650_000),
        "mapped rate joins must avoid repeated whole-child discovery: {work:?}"
    );
}

#[test]
fn range_rate_joins_avoid_repeating_full_child_discovery() {
    let mut admitted = 0;
    for function in crate::frontend::functions::FUNCTIONS {
        let expected = function.name != "absent_over_time"
            && matches!(
                RangeFunction::parse(function.name),
                Some(RangeFunction::Rate(_) | RangeFunction::Fold(_))
            );
        assert_eq!(
            CompiledQuery::is_replay_window_function(function.name),
            expected,
            "replay admission must match the typed owner for {}",
            function.name
        );
        admitted += usize::from(expected);
    }
    assert_eq!(admitted, 21);
    assert!(!CompiledQuery::is_replay_window_function(
        "absent_over_time"
    ));
    assert!(!CompiledQuery::is_replay_window_function(
        "quantile_over_time"
    ));
    let mut work = Vec::new();
    for function in ["increase", "delta", "irate", "idelta"] {
        for template in [
            "{f}(m_total[5m]) / on(instance) {f}(n_total[5m])",
            "clamp_min({f}(m_total[5m]),0) / on(instance) {f}(n_total[5m])",
            "{f}(m_total[5m]) / on(instance) abs({f}(n_total[5m]))",
            "abs(clamp_min({f}(m_total[5m]),-time())) / on(instance) abs({f}(n_total[5m]))",
            "(100 * +{f}(m_total[5m])) / on(instance) ({f}(n_total[5m]) * 100)",
        ] {
            let expression = template.replace("{f}", function);
            work.extend(window_join_discovery_cases(
                expression.as_bytes(),
                EngineOptions::default(),
            ));
        }
    }
    assert!(
        work.iter().all(|value| *value < 1_650_000),
        "range-rate joins must avoid repeated whole-child discovery: {work:?}"
    );
}

#[test]
fn per_series_fold_joins_avoid_repeating_full_child_discovery() {
    let mut options = EngineOptions::default();
    options.parser.experimental_functions = true;
    let mut work = Vec::new();
    for function in crate::frontend::functions::FUNCTIONS {
        if function.name == "absent_over_time"
            || !matches!(
                RangeFunction::parse(function.name),
                Some(RangeFunction::Fold(_))
            )
        {
            continue;
        }
        for template in [
            "({f}(m_total[5m])+1) / on(instance) ({f}(n_total[5m])+1)",
            "(clamp_min({f}(m_total[5m]),0)+1) / on(instance) ({f}(n_total[5m])+1)",
            "({f}(m_total[5m])+1) / on(instance) (abs({f}(n_total[5m]))+1)",
            "(abs(clamp_min({f}(m_total[5m]),-time()))+1) / on(instance) (abs({f}(n_total[5m]))+1)",
        ] {
            let expression = template.replace("{f}", function.name);
            work.extend(window_join_discovery_cases(expression.as_bytes(), options));
        }
    }
    assert_eq!(work.len(), 16 * 4 * 3 * 2);
    assert!(
        work.iter().all(|value| *value < 1_650_000),
        "per-series fold joins must avoid repeated whole-child discovery: {work:?}"
    );
}

#[test]
fn direct_rate_join_preserves_membership_order_and_mixed_operands() {
    fn point<const LEFT: bool, const ID: usize>(index: usize) -> crate::value::Sample {
        let missing = index == 2 && ((LEFT && ID == 0) || (!LEFT && ID == 1));
        crate::value::Sample {
            time_ms: (index as i64 + 1) * 1000,
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(if missing {
                f64::from_bits(crate::value::STALE_NAN_BITS)
            } else {
                (index as f64 + 1.0) * if LEFT { (1usize << ID) as f64 } else { 1.0 }
            }),
        }
    }
    let cases = [
        (
            b"rate(a_total[2s]) / on(instance) rate(b_total[2s])".as_slice(),
            [
                vec![(2000, 1.0), (5000, 1.0)],
                vec![(2000, 2.0), (5000, 2.0)],
                vec![(2000, 4.0), (3000, 4.0), (4000, 4.0), (5000, 4.0)],
            ],
        ),
        (
            b"rate(a_total[2s]) > on(instance) rate(b_total[2s])".as_slice(),
            [
                vec![],
                vec![(2000, 2.0), (5000, 2.0)],
                vec![(2000, 4.0), (3000, 4.0), (4000, 4.0), (5000, 4.0)],
            ],
        ),
        (
            b"rate(a_total[2s]) > bool on(instance) rate(b_total[2s])".as_slice(),
            [
                vec![(2000, 0.0), (5000, 0.0)],
                vec![(2000, 1.0), (5000, 1.0)],
                vec![(2000, 1.0), (3000, 1.0), (4000, 1.0), (5000, 1.0)],
            ],
        ),
        (
            b"a_total / on(instance) rate(b_total[2s])".as_slice(),
            [
                vec![(2000, 2.0), (4000, 4.0), (5000, 5.0)],
                vec![(2000, 4.0), (5000, 10.0)],
                vec![(2000, 8.0), (3000, 12.0), (4000, 16.0), (5000, 20.0)],
            ],
        ),
        (
            b"rate(a_total[2s]) / on(instance) b_total".as_slice(),
            [
                vec![(2000, 0.5), (5000, 0.2)],
                vec![(2000, 1.0), (4000, 0.5), (5000, 0.4)],
                vec![(2000, 2.0), (3000, 4.0 / 3.0), (4000, 1.0), (5000, 0.8)],
            ],
        ),
    ];
    for page_size in [1, 17, 1024] {
        for delayed in [false, true] {
            for (expression, expected) in &cases {
                let mut provider = FixtureProvider::new(5, page_size, 6);
                provider.filter_selectors = true;
                let samples: [fn(usize) -> crate::value::Sample; 6] = [
                    point::<true, 0>,
                    point::<true, 1>,
                    point::<true, 2>,
                    point::<false, 2>,
                    point::<false, 1>,
                    point::<false, 0>,
                ];
                for (index, series) in provider.series.iter_mut().enumerate() {
                    series.labels = vec![
                        (
                            b"__name__".to_vec(),
                            if index < 3 { b"a_total" } else { b"b_total" }.to_vec(),
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
                        start_ms: 2000,
                        end_ms: 5000,
                        step_ms: 1000,
                    },
                    &control,
                );
                let result = report.result.as_ref().unwrap();
                assert!(report.annotations.is_empty());
                assert_eq!(
                    result.series.len(),
                    expected.iter().filter(|points| !points.is_empty()).count()
                );
                let mut seen = std::collections::BTreeSet::new();
                for series in &*result.series {
                    assert_eq!(series.labels.0.len(), 1);
                    assert_eq!(&*series.labels.0[0].name, b"instance");
                    let id = std::str::from_utf8(&series.labels.0[0].value)
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    assert!(seen.insert(id));
                    let points: Vec<_> = series
                        .points
                        .iter()
                        .map(|point| {
                            let OwnedValue::Float(value) = point.value else {
                                panic!("float rate join");
                            };
                            (point.time_ms, value.to_bits())
                        })
                        .collect();
                    assert_eq!(
                        points,
                        expected[id]
                            .iter()
                            .map(|(time, value)| (*time, f64::to_bits(*value)))
                            .collect::<Vec<_>>(),
                        "page={page_size}, delayed={delayed}, expression={expression:?}, id={id}"
                    );
                }
                assert_eq!(
                    seen,
                    expected
                        .iter()
                        .enumerate()
                        .filter_map(|(id, points)| (!points.is_empty()).then_some(id))
                        .collect()
                );
                provider.assert_closed();
                drop((report, query));
                engine.accounting.snapshot().assert_memory_released();
            }
        }
    }
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
