// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    evaluator::EvaluationError,
    execution::{Engine, EngineOptions, ExecutionError, QueryRequest},
    result::CollectedResult,
    storage::{Control, LabelRef, StorageError, validate_labels},
    test_storage::FixtureProvider,
};
use std::sync::atomic::Ordering;

const BOUNDARY: usize = 1 << 24;
const MESSAGE: &[u8] = b"String too long to encode as label.";

fn runtime_error(result: Result<CollectedResult, ExecutionError>) {
    match result {
        Err(ExecutionError::Evaluation(EvaluationError::Runtime(message))) => {
            assert_eq!(&*message, MESSAGE);
        }
        Err(error) => panic!("expected label encoding error, got {error:?}"),
        Ok(_) => panic!("oversized materialized label was accepted"),
    }
}

#[test]
fn provider_field_boundaries_are_byte_lengths_and_not_labelset_lengths() {
    for length in [BOUNDARY - 1, BOUNDARY, BOUNDARY + 1] {
        let bytes = vec![0xff; length];
        for labels in [
            [LabelRef {
                name: &bytes,
                value: b"v",
            }],
            [LabelRef {
                name: b"a",
                value: &bytes,
            }],
        ] {
            match validate_labels(&labels) {
                Ok(()) if length < BOUNDARY => (),
                Err(StorageError::Invalid(message)) if length >= BOUNDARY => {
                    assert_eq!(message.as_bytes(), MESSAGE);
                }
                _ => panic!("incorrect provider label boundary at {length}"),
            }
        }
    }
    let bytes = vec![b'v'; BOUNDARY - 1];
    assert!(
        validate_labels(&[
            LabelRef {
                name: b"a",
                value: &bytes
            },
            LabelRef {
                name: b"b",
                value: &bytes
            },
        ])
        .is_ok()
    );
}

#[test]
fn oversized_provider_metadata_releases_owners_before_any_raw_cursor() {
    let engine = Engine::default();
    let control = Control::default();
    let query = engine
        .compile(b"counter_total", EngineOptions::default(), &control)
        .unwrap();
    for oversized_name in [false, true] {
        let mut provider = FixtureProvider::new(1, 1, 1);
        let field = &mut provider.series[0].labels[1];
        if oversized_name {
            field.0 = vec![b'a'; BOUNDARY];
        } else {
            field.1 = vec![b'v'; BOUNDARY];
        }
        let report =
            query.collect_reported(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
        assert!(matches!(
            report.result,
            Err(ExecutionError::Evaluation(EvaluationError::Storage(StorageError::Invalid(message))))
                if message.as_bytes() == MESSAGE
        ));
        assert_eq!(
            provider.series[0]
                .source
                .counters
                .opened
                .load(Ordering::Relaxed),
            0
        );
        provider.assert_closed();
    }
    drop(query);
    engine.accounting.snapshot().assert_memory_released();
}

#[test]
fn label_operations_check_only_the_materialized_fields() {
    let engine = Engine::default();
    let control = Control::default();
    for added in [0, 1, 2] {
        let mut provider = FixtureProvider::new(1, 1, 1);
        provider.series[0].labels[2].1 = vec![b'v'; BOUNDARY - 1];
        let separator = "x".repeat(added);
        let source =
            format!("label_join(counter_total,\"out\",\"{separator}\",\"job\",\"missing\")");
        let query = engine
            .compile(source.as_bytes(), EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&provider, QueryRequest::Instant { time_ms: 1000 }, &control);
        if added == 0 {
            let output = report.result.unwrap();
            assert_eq!(output.series.len(), 1);
            assert!(
                output.series[0]
                    .labels
                    .0
                    .iter()
                    .any(|label| { &*label.name == b"out" && label.value.len() == BOUNDARY - 1 })
            );
        } else {
            runtime_error(report.result);
        }
        provider.assert_closed();
    }
    engine.accounting.snapshot().assert_memory_released();
}

#[test]
fn absence_materializes_after_provisional_set_and_delete() {
    let engine = Engine::default();
    let control = Control::default();
    let provider = FixtureProvider::new(0, 1, 0);
    let value = "v".repeat(BOUNDARY);
    for remove in [false, true] {
        let source = format!(
            "absent(m{{k=\"{value}\"{}}})",
            if remove { ",k=\"x\"" } else { "" }
        );
        let query = engine
            .compile(source.as_bytes(), EngineOptions::default(), &control)
            .unwrap();
        let report =
            query.collect_reported(&provider, QueryRequest::Instant { time_ms: 0 }, &control);
        if remove {
            let output = report.result.unwrap();
            assert_eq!(output.series.len(), 1);
            assert!(output.series[0].labels.0.is_empty());
            assert_eq!(output.series[0].points.len(), 1);
        } else {
            runtime_error(report.result);
        }
        provider.assert_closed();
    }
    engine.accounting.snapshot().assert_memory_released();
}

#[test]
fn generated_names_are_validated_before_count_grouping() {
    let engine = Engine::default();
    let control = Control::default();
    let provider = FixtureProvider::new(1, 1, 1);
    for length in [BOUNDARY - 1, BOUNDARY, BOUNDARY + 1] {
        let name = "a".repeat(length);
        for count in [false, true] {
            let source = if count {
                format!("count_values without (\"{name}\") (\"{name}\", counter_total)")
            } else {
                format!("label_join(counter_total,\"{name}\",\"\",\"job\")")
            };
            let query = engine
                .compile(source.as_bytes(), EngineOptions::default(), &control)
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 1000 },
                &control,
            );
            if length < BOUNDARY {
                let output = report.result.unwrap();
                assert_eq!(output.series.len(), 1);
                assert_eq!(
                    output.series[0]
                        .labels
                        .0
                        .iter()
                        .any(|label| label.name.len() == length),
                    !count
                );
            } else {
                runtime_error(report.result);
            }
            provider.assert_closed();
        }
    }
    engine.accounting.snapshot().assert_memory_released();
}

#[test]
fn replacements_validate_generated_fields_but_allow_unmatched_strings_and_deletions() {
    let engine = Engine::default();
    let control = Control::default();
    let provider = FixtureProvider::new(1, 1, 1);
    for length in [BOUNDARY - 1, BOUNDARY, BOUNDARY + 1] {
        let bytes = "a".repeat(length);
        for action in 0..3 {
            let source = match action {
                0 => format!("label_replace(counter_total,\"out\",\"{bytes}\",\"job\",\"worker\")"),
                1 => {
                    format!("label_replace(counter_total,\"out\",\"{bytes}\",\"job\",\"missing\")")
                }
                _ => format!("label_replace(counter_total,\"{bytes}\",\"\",\"job\",\"worker\")"),
            };
            let query = engine
                .compile(source.as_bytes(), EngineOptions::default(), &control)
                .unwrap();
            let report = query.collect_reported(
                &provider,
                QueryRequest::Instant { time_ms: 1000 },
                &control,
            );
            if action == 0 && length >= BOUNDARY {
                runtime_error(report.result);
            } else {
                let output = report.result.unwrap();
                assert_eq!(output.series.len(), 1);
                assert_eq!(
                    output.series[0]
                        .labels
                        .0
                        .iter()
                        .any(|label| &*label.name == b"out"),
                    action == 0
                );
                if action == 0 {
                    assert_eq!(
                        output.series[0]
                            .labels
                            .0
                            .iter()
                            .find(|label| &*label.name == b"out")
                            .unwrap()
                            .value
                            .len(),
                        length
                    );
                }
            }
            provider.assert_closed();
        }
    }
    engine.accounting.snapshot().assert_memory_released();
}
