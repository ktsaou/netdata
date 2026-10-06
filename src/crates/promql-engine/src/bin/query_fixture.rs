// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only immutable snapshots; engine reads remain bounded borrowed pages.
use std::io::{self, BufRead, Write};

use netdata_promql_engine::{
    accounting::{MemoryKind, QueryObservations},
    annotations::{Annotations, Severity},
    evaluator::EvaluationError,
    execution::{CompileError, Engine, EngineOptions, ExecutionError, QueryRequest},
    frontend::ast::{ParseError, ParserOptions, ValueType},
    result::{CollectedResult, OwnedValue},
    storage::{
        Control, LabelRef, PageSamples, Provider, ProviderResources, QueryProvider, SampleCursor,
        SamplePage, SelectionRequest, Series, SeriesSelection, StorageError, TimeBounds,
    },
    value::{Sample, SampleValue},
};
use serde_json::{Value, json};

pub mod fixture_codec;
use fixture_codec::{
    Error, decoded_bytes, encoded_bytes, encoded_float_ref, float, integer, number,
};

struct FixtureSeries {
    labels: Vec<(Vec<u8>, Vec<u8>)>,
    points: Vec<Sample>,
}
struct FixtureProvider {
    id: String,
    series: Vec<FixtureSeries>,
    page_size: usize,
}
impl FixtureProvider {
    fn decode(request: &Value) -> Result<Self, Error> {
        let data = &request["dataset"];
        if data["schema_version"].as_u64() != Some(1) {
            return Err("invalid fixture dataset schema".into());
        }
        let mut series = Vec::new();
        for input in data["series"].as_array().ok_or("missing series")? {
            let mut labels = input["labels"]
                .as_array()
                .ok_or("missing labels")?
                .iter()
                .map(|label| {
                    Ok((
                        decoded_bytes(&label["name"])?,
                        decoded_bytes(&label["value"])?,
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            labels.sort_by(|a, b| a.0.cmp(&b.0));
            if labels.iter().any(|label| label.0.is_empty())
                || labels.windows(2).any(|pair| pair[0].0 == pair[1].0)
            {
                return Err("invalid fixture labels".into());
            }
            let mut points = Vec::new();
            for point in input["points"].as_array().ok_or("missing points")? {
                let value = match point["kind"].as_str() {
                    Some("float") => SampleValue::Float(number(&point["float"])?),
                    Some("histogram") => {
                        SampleValue::IntegerHistogram(integer(&point["histogram"])?)
                    }
                    Some("float_histogram") => {
                        SampleValue::FloatHistogram(float(&point["float_histogram"])?)
                    }
                    _ => return Err("invalid fixture sample kind".into()),
                };
                points.push(Sample {
                    time_ms: point["time_ms"]
                        .as_i64()
                        .ok_or("invalid sample timestamp")?,
                    start_time_ms: point
                        .get("start_time_ms")
                        .map_or(Ok(0), |v| v.as_i64().ok_or("invalid start timestamp"))?,
                    value,
                });
            }
            if points
                .windows(2)
                .any(|pair| pair[0].time_ms >= pair[1].time_ms)
            {
                return Err("fixture timestamps are not strictly increasing".into());
            }
            series.push(FixtureSeries { labels, points });
        }
        let page_size = request["page_size"]
            .as_u64()
            .ok_or("missing page size")?
            .try_into()?;
        if page_size == 0 {
            return Err("fixture pages cannot be empty".into());
        }
        Ok(Self {
            id: request["dataset_id"]
                .as_str()
                .ok_or("missing dataset id")?
                .into(),
            series,
            page_size,
        })
    }
}
struct FixtureQuery<'a>(&'a FixtureProvider);
struct FixtureSelection<'a> {
    provider: &'a FixtureProvider,
    index: usize,
}
struct FixtureHandle<'a> {
    series: &'a FixtureSeries,
    page_size: usize,
}
struct FixtureCursor<'a> {
    points: &'a [Sample],
    index: usize,
    page_size: usize,
}
impl Provider for FixtureProvider {
    type Query<'a> = FixtureQuery<'a>;
    fn open(
        &self,
        _bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Query<'_>, StorageError> {
        control.check()?;
        Ok(FixtureQuery(self))
    }
}
impl QueryProvider for FixtureQuery<'_> {
    type Selection<'a>
        = FixtureSelection<'a>
    where
        Self: 'a;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn select(
        &self,
        _: SelectionRequest<'_>,
        control: &Control,
    ) -> Result<Self::Selection<'_>, StorageError> {
        control.check()?;
        // Overscan metadata only; the engine authoritatively applies matchers and time bounds.
        Ok(FixtureSelection {
            provider: self.0,
            index: 0,
        })
    }
}
impl<'a> SeriesSelection for FixtureSelection<'a> {
    type Series = FixtureHandle<'a>;
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn next(&mut self, control: &Control) -> Result<Option<Self::Series>, StorageError> {
        control.check()?;
        let Some(series) = self.provider.series.get(self.index) else {
            return Ok(None);
        };
        self.index += 1;
        Ok(Some(FixtureHandle {
            series,
            page_size: self.provider.page_size,
        }))
    }
}
impl Series for FixtureHandle<'_> {
    type Cursor<'a>
        = FixtureCursor<'a>
    where
        Self: 'a;
    fn label_count(&self) -> usize {
        self.series.labels.len()
    }
    fn label(&self, index: usize) -> LabelRef<'_> {
        let (name, value) = &self.series.labels[index];
        LabelRef { name, value }
    }
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn cursor(
        &self,
        bounds: TimeBounds,
        control: &Control,
    ) -> Result<Self::Cursor<'_>, StorageError> {
        control.check()?;
        let points = &self.series.points;
        let start = points.partition_point(|p| p.time_ms < bounds.start_ms);
        let end = points.partition_point(|p| p.time_ms <= bounds.end_ms);
        Ok(FixtureCursor {
            points: &points[start..end.max(start)],
            index: 0,
            page_size: self.page_size,
        })
    }
}
impl SampleCursor for FixtureCursor<'_> {
    fn resources(&self) -> ProviderResources {
        ProviderResources::default()
    }
    fn max_page_samples(&self) -> usize {
        self.page_size
    }
    fn seek(&mut self, time_ms: i64, control: &Control) -> Result<(), StorageError> {
        control.check()?;
        self.index = self.points.partition_point(|p| p.time_ms < time_ms);
        Ok(())
    }
    fn next(
        &mut self,
        control: &Control,
        resources: &mut ProviderResources,
    ) -> Result<Option<SamplePage<'_>>, StorageError> {
        *resources = self.resources();
        control.check()?;
        if self.index == self.points.len() {
            return Ok(None);
        }
        let start = self.index;
        self.index = self.points.len().min(start.saturating_add(self.page_size));
        Ok(Some(SamplePage {
            samples: PageSamples::Samples(&self.points[start..self.index]),
            resources: *resources,
        }))
    }
}

fn messages(annotations: &Annotations, severity: Severity) -> Vec<String> {
    let mut messages: Vec<_> = annotations
        .iter()
        .filter(|a| a.severity == severity)
        .map(|a| encoded_bytes(a.message))
        .collect();
    messages.sort();
    messages
}
fn result_rows(result: &CollectedResult) -> Vec<Value> {
    if let Some((time, value)) = result.scalar {
        return vec![
            json!({"labels": [], "points": [{"time_ms": time, "kind": "float", "float": format!("{:016x}", value.to_bits())}]}),
        ];
    }
    if let Some((time, value)) = &result.string {
        return vec![
            json!({"labels": [], "points": [{"time_ms": time, "kind": "string", "string": encoded_bytes(value)}]}),
        ];
    }
    result.series.iter().map(|row| {
        let labels: Vec<_> = row.labels.0.iter().map(|label| json!({"name": encoded_bytes(&label.name), "value": encoded_bytes(&label.value)})).collect();
        let points: Vec<_> = row.points.iter().map(|point| match &point.value {
            OwnedValue::Float(value) => json!({"time_ms": point.time_ms, "kind": "float", "float": format!("{:016x}", value.to_bits())}),
            OwnedValue::Histogram(value) => json!({"time_ms": point.time_ms, "kind": "float_histogram", "float_histogram": encoded_float_ref(value.as_ref())}),
        }).collect();
        json!({"labels": labels, "points": points})
    }).collect()
}
fn observations(observed: &QueryObservations) -> Value {
    let memory: Vec<_> = observed.memory.iter().map(|m| json!({"current": m.current, "peak": m.peak, "acquired": m.acquired, "released": m.released, "overflowed": m.overflowed})).collect();
    let references = |value: netdata_promql_engine::accounting::ReferenceObservation| json!({"current": value.current, "peak": value.peak, "acquired": value.acquired, "released": value.released, "overflowed": value.overflowed});
    json!({"memory": memory, "cancellation_references": {"engine_owned": references(observed.cancellation_references.engine_owned), "external": references(observed.cancellation_references.external)}, "pages": observed.pages, "samples_fetched": observed.samples_fetched, "work": observed.work, "provider_calls": observed.provider_calls, "output_samples": observed.output_samples, "elapsed_ns": observed.elapsed.as_nanos().to_string(), "counter_overflowed": observed.counter_overflowed})
}
fn failure(phase: &str, kind: &str, message: &[u8]) -> Value {
    json!({"phase": phase, "type": kind, "message": encoded_bytes(message)})
}
fn parse_failure(source: &[u8], errors: &[ParseError]) -> Value {
    let first = &errors[0];
    let position = if source.is_empty() {
        "unknown position".into()
    } else if first.span.start > source.len() {
        "invalid position".into()
    } else {
        let prefix = &source[..first.span.start];
        let line = 1 + prefix.iter().filter(|&&b| b == b'\n').count();
        let column = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(first.span.start + 1, |i| first.span.start - i);
        format!("{line}:{column}")
    };
    let mut message = format!("{position}: parse error: ").into_bytes();
    message.extend_from_slice(&first.message);
    let mut result = failure("construction", "parser.ParseErrors", &message);
    result["parse_errors"] = json!(errors.iter().map(|e| json!({"start": e.span.start, "end": e.span.end, "line_offset": 0, "message": encoded_bytes(&e.message)})).collect::<Vec<_>>());
    result
}
fn evaluate(provider: &FixtureProvider, request: &Value) -> Result<Value, Error> {
    let id = request["id"].as_str().ok_or("missing query id")?;
    if request["dataset_id"].as_str() != Some(&provider.id) {
        return Err("fixture dataset identity differs".into());
    }
    let source = decoded_bytes(&request["expression"])?;
    let profile = request["profile"].as_str().ok_or("missing profile")?;
    let extended = match profile {
        "server-default" => false,
        "upstream-tests" => true,
        _ => return Err("unknown fixture profile".into()),
    };
    let options = EngineOptions {
        parser: ParserOptions {
            experimental_functions: extended,
            binop_fill_modifiers: extended,
            extended_range_selectors: extended,
            duration_expressions: true,
        },
        lookback_ms: request
            .get("lookback_ms")
            .map_or(Ok(300_000), |v| v.as_i64().ok_or("invalid lookback"))?,
        no_step_subquery_interval_ms: request
            .get("subquery_step_ms")
            .map_or(Ok(60_000), |v| v.as_i64().ok_or("invalid no-step interval"))?,
        start_timestamps: extended,
        delayed_name_removal: extended,
    };
    let query_request = match request["mode"].as_str() {
        Some("instant") => QueryRequest::Instant {
            time_ms: request
                .get("time_ms")
                .map_or(Ok(0), |v| v.as_i64().ok_or("invalid instant time"))?,
        },
        Some("range") => QueryRequest::Range {
            start_ms: request["start_ms"].as_i64().ok_or("invalid range start")?,
            end_ms: request["end_ms"].as_i64().ok_or("invalid range end")?,
            step_ms: request["step_ms"].as_i64().ok_or("invalid range step")?,
        },
        _ => return Err("unknown fixture query mode".into()),
    };
    let engine = Engine::default();
    let baseline = engine.accounting.snapshot();
    let control = Control::default();
    let mut result = json!({"id": id, "kind": "error", "rows": [], "warnings": [], "infos": []});
    let query = match engine.compile(&source, options, &control) {
        Ok(query) => query,
        Err(CompileError::Parse(errors)) => {
            result["error"] = parse_failure(&source, &errors);
            if request["op"].as_str() == Some("prepare") {
                result["error"]["phase"] = json!("parse");
                return Ok(json!({"id": id, "scope": {"error": result["error"]}}));
            }
            return Ok(json!({"id": id, "result": result}));
        }
        Err(error) => return Err(format!("fixture compilation failed: {error:?}").into()),
    };
    if request["op"].as_str() == Some("prepare") {
        let scope = match query.preparation_fixture(query_request, &control) {
            Ok(snapshot) => snapshot,
            Err(EvaluationError::Preparation(message)) => {
                json!({"error": failure("preprocess", "*errors.errorString", &message)})
            }
            Err(error) => return Err(format!("fixture preparation failed: {error:?}").into()),
        };
        return Ok(json!({"id": id, "scope": scope}));
    }
    if request["op"].as_str() == Some("select-plan") {
        let scope = query
            .selection_fixture(query_request, &control)
            .map_err(|error| format!("fixture selection projection failed: {error:?}"))?;
        return Ok(json!({"id": id, "scope": scope}));
    }
    if matches!(query_request, QueryRequest::Range { .. })
        && !matches!(query.value_type(), ValueType::Scalar | ValueType::Vector)
    {
        result["error"] = failure(
            "construction",
            "*errors.errorString",
            format!(
                "invalid expression type \"{}\" for range query, must be Scalar or instant Vector",
                query.value_type().diagnostic_name()
            )
            .as_bytes(),
        );
        return Ok(json!({"id": id, "result": result}));
    }
    let report = query.collect_reported(provider, query_request, &control);
    let observed = observations(&report.observations);
    result["warnings"] = json!(messages(&report.annotations, Severity::Warning));
    result["infos"] = json!(messages(&report.annotations, Severity::Info));
    let mut unsupported = None;
    match report.result {
        Ok(output) => {
            result["kind"] = json!(output.header.value_type.name());
            result["rows"] = json!(result_rows(&output));
        }
        Err(ExecutionError::Evaluation(EvaluationError::Unsupported(reason))) => {
            unsupported = Some(encoded_bytes(reason.as_bytes()));
        }
        Err(ExecutionError::Evaluation(EvaluationError::Invalid(message))) => {
            result["error"] = failure("execution", "*errors.errorString", message.as_bytes());
        }
        Err(ExecutionError::Evaluation(EvaluationError::Preparation(message))) => {
            result["error"] = failure("construction", "*errors.errorString", &message);
        }
        Err(ExecutionError::Evaluation(EvaluationError::Runtime(message))) => {
            result["error"] = failure("execution", "*errors.errorString", &message);
        }
        Err(ExecutionError::Evaluation(EvaluationError::Expansion(message))) => {
            result["error"] = failure("execution", "*fmt.wrapError", &message);
        }
        Err(error) => return Err(format!("fixture execution failed: {error:?}").into()),
    }
    drop(query);
    drop(report.annotations);
    // Owned output is projected and released before checking query-owned storage cleanup.
    let snapshot = engine.accounting.snapshot();
    if snapshot.active_executions != 0
        || snapshot.memory.iter().enumerate().any(|(index, m)| {
            index != MemoryKind::SharedCacheReference as usize
                && m.current != baseline.memory[index].current
        })
        || [
            snapshot.cancellation_references.engine_owned,
            snapshot.cancellation_references.external,
        ]
        .iter()
        .any(|reference| {
            reference.current != 0
                || reference.acquired != reference.released
                || reference.overflowed
        })
    {
        return Err("fixture query leaked an observed owner".into());
    }
    Ok(match unsupported {
        Some(reason) => json!({"id": id, "unsupported": reason, "observations": observed}),
        None => json!({"id": id, "result": result, "observations": observed}),
    })
}
fn main() -> Result<(), Error> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut provider = None;
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let response = if request["op"].as_str() == Some("dataset") {
            let next = FixtureProvider::decode(&request)?;
            let response = json!({"dataset_id": next.id});
            provider = Some(next);
            response
        } else {
            evaluate(
                provider.as_ref().ok_or("no loaded fixture dataset")?,
                &request,
            )?
        };
        serde_json::to_writer(&mut out, &response)?;
        writeln!(out)?;
    }
    out.flush()?;
    Ok(())
}
