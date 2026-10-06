// SPDX-License-Identifier: GPL-3.0-or-later
//! Fixtures-only observations of generated and opaque annotation owners.
use super::{AnnotationBuilder, AnnotationKind, Monotonicity, Severity};
use crate::{
    accounting::{EngineAccounting, MemoryKind},
    frontend::Span,
    storage::{Control, ProviderDiagnosticRef},
};
use serde_json::{Value, json};

fn bytes(value: &Value, key: &str) -> Result<Vec<u8>, String> {
    let text = value.get(key).and_then(Value::as_str).unwrap_or("");
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return Err("invalid fixture hex bytes".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
fn float(value: &Value, key: &str) -> Result<f64, String> {
    let bytes: [u8; 8] = bytes(value, key)?
        .try_into()
        .map_err(|_| "expected binary64 bytes")?;
    Ok(f64::from_bits(u64::from_be_bytes(bytes)))
}
fn integer(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for byte in bytes {
        write!(out, "{byte:02x}").expect("string writer");
    }
    out
}

pub fn result(id: &str, input: &[u8]) -> Result<Value, String> {
    let request: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    if request.get("id").and_then(Value::as_str) != Some(id) {
        return Err("annotation fixture identity differs".into());
    }
    let events = request
        .get("events")
        .and_then(Value::as_array)
        .ok_or("missing annotation events")?;
    let query = bytes(&request, "query")?;
    let engine = EngineAccounting::default();
    let account = engine.query();
    let control = Control::default();
    let mut parent = AnnotationBuilder::new(&account);
    let mut child = AnnotationBuilder::new(&account);
    for event in events {
        let operation = event.get("op").and_then(Value::as_str);
        if operation == Some("merge") {
            parent.merge(child, &query);
            child = AnnotationBuilder::new(&account);
            continue;
        }
        let builder = match operation {
            Some("add") => &mut parent,
            Some("child_add") => &mut child,
            _ => return Err("invalid annotation operation".into()),
        };
        let span = Span {
            start: integer(event, "start") as usize,
            end: integer(event, "end") as usize,
        };
        let metric = bytes(event, "metric")?;
        match event.get("kind").and_then(Value::as_str) {
            Some("monotonic") => builder.add_monotonicity(
                &metric,
                span,
                Monotonicity::new(
                    integer(event, "time_ms"),
                    float(event, "min")?,
                    float(event, "max")?,
                    float(event, "diff")?,
                ),
                &query,
                &account,
            ),
            Some("invalid_quantile") => {
                builder.add_invalid_quantile(float(event, "q")?, span, &query, &account);
            }
            Some("bad_bucket") => {
                builder.add_bad_bucket(&metric, &bytes(event, "label")?, span, &query, &account);
            }
            Some("opaque") => {
                let raw = bytes(event, "raw")?;
                let severity = match event.get("severity").and_then(Value::as_str) {
                    Some("info") => Severity::Info,
                    Some("warning") => Severity::Warning,
                    _ => return Err("invalid opaque severity".into()),
                };
                builder
                    .add_provider(
                        ProviderDiagnosticRef {
                            severity,
                            message: &raw,
                            position: (event.get("no_position").and_then(Value::as_bool)
                                != Some(true))
                            .then_some(span),
                        },
                        &query,
                        Some(&control),
                        &account,
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
            other => {
                let kind = match other {
                    Some("mixed_classic_native") => AnnotationKind::MixedClassicNativeHistograms,
                    Some("quantile_nan_result") => AnnotationKind::NativeHistogramQuantileNaNResult,
                    Some("quantile_nan_skew") => AnnotationKind::NativeHistogramQuantileNaNSkew,
                    Some("fraction_nans") => AnnotationKind::NativeHistogramFractionNaNs,
                    _ => return Err("unknown annotation kind".into()),
                };
                builder.add(kind, &metric, span, &query, &account);
            }
        }
    }
    drop(child);
    let result = parent.finish();
    let mut notes: Vec<_> = result
        .iter()
        .map(|note| {
            json!({
                "raw": hex(note.raw_message), "message": hex(note.message),
                "severity": if note.severity == Severity::Info { "info" } else { "warning" },
                "span": note.span.map(|span| json!({"start": span.start as isize, "end": span.end as isize})),
                "occurrences": note.occurrences,
            })
        })
        .collect();
    notes.sort_by(|a, b| a["raw"].as_str().cmp(&b["raw"].as_str()));
    let held = account.snapshot().memory[MemoryKind::Result as usize].current;
    let retained = result.clone();
    if account.snapshot().memory[MemoryKind::Result as usize].current != held {
        return Err("aliased notes changed allocation ownership".into());
    }
    drop(result);
    if account.snapshot().memory[MemoryKind::Result as usize].current != held {
        return Err("retained notes lost allocation ownership".into());
    }
    drop(retained);
    let released = account.snapshot().memory[MemoryKind::Result as usize];
    Ok(json!({
        "id": id, "notes": notes,
        "accounting": {
            "result_held": held, "result_peak": released.peak,
            "result_current_after_drop": released.current,
            "result_acquired": released.acquired, "result_released": released.released,
        },
    }))
}
