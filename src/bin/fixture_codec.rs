// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared test-only byte, binary64 and typed histogram transport.
use netdata_promql_engine::histogram::{FloatHistogram, FloatHistogramRef, IntegerHistogram, Span};
use serde_json::{Value, json};

pub type Error = Box<dyn std::error::Error>;

pub fn number(value: &Value) -> Result<f64, Error> {
    let text = value.as_str().ok_or("expected binary64 hex string")?;
    if text.len() != 16 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid binary64 hex string".into());
    }
    Ok(f64::from_bits(u64::from_str_radix(text, 16)?))
}

pub fn decoded_bytes(value: &Value) -> Result<Vec<u8>, Error> {
    let text = value.as_str().ok_or("expected hex bytes")?;
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return Err("invalid hex bytes".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|index| Ok(u8::from_str_radix(&text[index..index + 2], 16)?))
        .collect()
}

pub fn encoded_bytes(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn array<T>(
    value: &Value,
    parse: impl Fn(&Value) -> Result<T, Error>,
) -> Result<Option<Vec<T>>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    let values = value.as_array().ok_or("expected array or null")?;
    Ok(Some(values.iter().map(parse).collect::<Result<_, _>>()?))
}

pub fn span(value: &Value) -> Result<Span, Error> {
    Ok(Span {
        offset: value["Offset"]
            .as_i64()
            .ok_or("invalid span offset")?
            .try_into()?,
        length: value["Length"]
            .as_u64()
            .ok_or("invalid span length")?
            .try_into()?,
    })
}

pub fn float(value: &Value) -> Result<FloatHistogram, Error> {
    Ok(FloatHistogram {
        reset_hint: value["reset_hint"]
            .as_u64()
            .ok_or("invalid reset hint")?
            .try_into()?,
        schema: value["schema"]
            .as_i64()
            .ok_or("invalid schema")?
            .try_into()?,
        zero_threshold: number(&value["zero_threshold"])?,
        zero_count: number(&value["zero_count"])?,
        count: number(&value["count"])?,
        sum: number(&value["sum"])?,
        positive_spans: array(&value["positive_spans"], span)?.map(Into::into),
        negative_spans: array(&value["negative_spans"], span)?.map(Into::into),
        positive_buckets: array(&value["positive_buckets"], number)?.map(Into::into),
        negative_buckets: array(&value["negative_buckets"], number)?.map(Into::into),
        custom_values: array(&value["custom_values"], number)?.map(Into::into),
    })
}

pub fn integer(value: &Value) -> Result<IntegerHistogram, Error> {
    let bucket = |value: &Value| Ok(value.as_i64().ok_or("invalid integer bucket")?);
    Ok(IntegerHistogram {
        reset_hint: value["reset_hint"]
            .as_u64()
            .ok_or("invalid reset hint")?
            .try_into()?,
        schema: value["schema"]
            .as_i64()
            .ok_or("invalid schema")?
            .try_into()?,
        zero_threshold: number(&value["zero_threshold"])?,
        zero_count: value["zero_count"]
            .as_u64()
            .ok_or("invalid integer zero count")?,
        count: value["count"].as_u64().ok_or("invalid integer count")?,
        sum: number(&value["sum"])?,
        positive_spans: array(&value["positive_spans"], span)?,
        negative_spans: array(&value["negative_spans"], span)?,
        positive_buckets: array(&value["positive_buckets"], bucket)?,
        negative_buckets: array(&value["negative_buckets"], bucket)?,
        custom_values: array(&value["custom_values"], number)?.map(Into::into),
    })
}

pub fn encoded_numbers(values: Option<&[f64]>) -> Value {
    values.map_or(Value::Null, |values| {
        json!(
            values
                .iter()
                .map(|value| format!("{:016x}", value.to_bits()))
                .collect::<Vec<_>>()
        )
    })
}

pub fn encoded_spans(values: Option<&[Span]>) -> Value {
    values.map_or(Value::Null, |values| {
        json!(
            values
                .iter()
                .map(|span| json!({"Offset": span.offset, "Length": span.length}))
                .collect::<Vec<_>>()
        )
    })
}

pub fn encoded_float_ref(value: FloatHistogramRef<'_>) -> Value {
    json!({"reset_hint": value.reset_hint, "schema": value.schema,
        "zero_threshold": format!("{:016x}", value.zero_threshold.to_bits()),
        "zero_count": format!("{:016x}", value.zero_count.to_bits()),
        "count": format!("{:016x}", value.count.to_bits()), "sum": format!("{:016x}", value.sum.to_bits()),
        "positive_spans": encoded_spans(value.positive_spans),
        "negative_spans": encoded_spans(value.negative_spans),
        "positive_buckets": encoded_numbers(value.positive_buckets),
        "negative_buckets": encoded_numbers(value.negative_buckets),
        "custom_values": encoded_numbers(value.custom_values)})
}

pub fn encoded_float(value: &FloatHistogram) -> Value {
    encoded_float_ref(value.as_ref())
}

pub fn encoded_query_float_ref(mut value: FloatHistogramRef<'_>) -> Value {
    // The query oracle copies spans with append(nil, spans...), unlike native probes.
    value.positive_spans = value.positive_spans.filter(|spans| !spans.is_empty());
    value.negative_spans = value.negative_spans.filter(|spans| !spans.is_empty());
    encoded_float_ref(value)
}

pub fn encoded_integer(value: &IntegerHistogram) -> Value {
    json!({"reset_hint": value.reset_hint, "schema": value.schema,
        "zero_threshold": format!("{:016x}", value.zero_threshold.to_bits()),
        "zero_count": value.zero_count, "count": value.count, "sum": format!("{:016x}", value.sum.to_bits()),
        "positive_spans": encoded_spans(value.positive_spans.as_deref()),
        "negative_spans": encoded_spans(value.negative_spans.as_deref()),
        "positive_buckets": value.positive_buckets, "negative_buckets": value.negative_buckets,
        "custom_values": encoded_numbers(value.custom_values.as_deref())})
}
