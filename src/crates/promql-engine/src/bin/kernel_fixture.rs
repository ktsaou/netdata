// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only per-sample kernel transport; it is not a storage/evaluation adapter.
use std::io::{self, BufRead, Write};

use netdata_promql_engine::histogram::{
    Bucket, FloatHistogram, IntegerHistogram, Span, bucket_bound,
};
use netdata_promql_engine::numerical::{compensated_add, compensated_sub};
use serde_json::{Value, json};

type Error = Box<dyn std::error::Error>;

fn number(value: &Value) -> Result<f64, Error> {
    let text = value.as_str().ok_or("expected binary64 hex string")?;
    if text.len() != 16 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid binary64 hex string".into());
    }
    Ok(f64::from_bits(u64::from_str_radix(text, 16)?))
}

fn array<T>(
    value: &Value,
    parse: impl Fn(&Value) -> Result<T, Error>,
) -> Result<Option<Vec<T>>, Error> {
    if value.is_null() {
        return Ok(None);
    }
    let values = value.as_array().ok_or("expected array or null")?;
    Ok(Some(values.iter().map(parse).collect::<Result<_, _>>()?))
}

fn span(value: &Value) -> Result<Span, Error> {
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

fn float(value: &Value) -> Result<FloatHistogram, Error> {
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
        positive_spans: array(&value["positive_spans"], span)?,
        negative_spans: array(&value["negative_spans"], span)?,
        positive_buckets: array(&value["positive_buckets"], number)?,
        negative_buckets: array(&value["negative_buckets"], number)?,
        custom_values: array(&value["custom_values"], number)?.map(Into::into),
    })
}

fn integer(value: &Value) -> Result<IntegerHistogram, Error> {
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

fn encoded_numbers(values: Option<&[f64]>) -> Value {
    values.map_or(Value::Null, |values| {
        json!(
            values
                .iter()
                .map(|value| format!("{:016x}", value.to_bits()))
                .collect::<Vec<_>>()
        )
    })
}

fn encoded_spans(values: Option<&[Span]>) -> Value {
    values.map_or(Value::Null, |values| {
        json!(
            values
                .iter()
                .map(|span| json!({"Offset": span.offset, "Length": span.length}))
                .collect::<Vec<_>>()
        )
    })
}

fn encoded_float(value: &FloatHistogram) -> Value {
    json!({"reset_hint": value.reset_hint, "schema": value.schema,
        "zero_threshold": format!("{:016x}", value.zero_threshold.to_bits()),
        "zero_count": format!("{:016x}", value.zero_count.to_bits()),
        "count": format!("{:016x}", value.count.to_bits()), "sum": format!("{:016x}", value.sum.to_bits()),
        "positive_spans": encoded_spans(value.positive_spans.as_deref()),
        "negative_spans": encoded_spans(value.negative_spans.as_deref()),
        "positive_buckets": encoded_numbers(value.positive_buckets.as_deref()),
        "negative_buckets": encoded_numbers(value.negative_buckets.as_deref()),
        "custom_values": encoded_numbers(value.custom_values.as_deref())})
}

fn encoded_integer(value: &IntegerHistogram) -> Value {
    json!({"reset_hint": value.reset_hint, "schema": value.schema,
        "zero_threshold": format!("{:016x}", value.zero_threshold.to_bits()),
        "zero_count": value.zero_count, "count": value.count, "sum": format!("{:016x}", value.sum.to_bits()),
        "positive_spans": encoded_spans(value.positive_spans.as_deref()),
        "negative_spans": encoded_spans(value.negative_spans.as_deref()),
        "positive_buckets": value.positive_buckets, "negative_buckets": value.negative_buckets,
        "custom_values": encoded_numbers(value.custom_values.as_deref())})
}

fn encoded_iterator(iterator: impl Iterator<Item = Bucket>) -> Value {
    let buckets: Vec<_> = iterator
        .map(|bucket| {
            json!({"lower": format!("{:016x}", bucket.lower.to_bits()),
            "upper": format!("{:016x}", bucket.upper.to_bits()),
            "count": format!("{:016x}", bucket.count.to_bits()), "index": bucket.index,
            "lower_inclusive": bucket.lower_inclusive, "upper_inclusive": bucket.upper_inclusive})
        })
        .collect();
    if buckets.is_empty() {
        Value::Null
    } else {
        json!(buckets)
    }
}

fn iterator_result(request: &Value, borrowed: bool) -> Result<Value, Error> {
    let histogram = float(&request["input"])?;
    let view = histogram.as_ref();
    let routes = request["routes"]
        .as_array()
        .ok_or("invalid iterator routes")?;
    let start = number(&request["start"])?;
    let target = request["target_schema"]
        .as_i64()
        .ok_or("invalid iterator target schema")?
        .try_into()?;
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Value, Error> {
            let mut buckets = serde_json::Map::new();
            for route in routes {
                let route = route.as_str().ok_or("invalid iterator route")?;
                let values = match route {
                    "all" | "all-reverse" => encoded_iterator(if borrowed {
                        view.all_bucket_iterator(route == "all-reverse")
                    } else {
                        histogram.all_bucket_iterator(route == "all-reverse")
                    }),
                    "positive" | "negative" => encoded_iterator(if borrowed {
                        view.bucket_iterator(route == "positive", 0.0, histogram.schema)
                    } else {
                        histogram.bucket_iterator(route == "positive", 0.0, histogram.schema)
                    }),
                    "positive-reverse" | "negative-reverse" => encoded_iterator(if borrowed {
                        view.reverse_bucket_iterator(route == "positive-reverse")
                    } else {
                        histogram.reverse_bucket_iterator(route == "positive-reverse")
                    }),
                    "low-positive" | "low-negative" => encoded_iterator(if borrowed {
                        view.bucket_iterator(route == "low-positive", start, target)
                    } else {
                        histogram.bucket_iterator(route == "low-positive", start, target)
                    }),
                    _ => return Err("unknown iterator route".into()),
                };
                if buckets.insert(route.to_owned(), values).is_some() {
                    return Err("duplicate iterator route".into());
                }
            }
            Ok(Value::Object(buckets))
        }));
    match result {
        Ok(buckets) => Ok(json!({"buckets": buckets?, "panic": ""})),
        Err(failure) => {
            let message = failure
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| {
                    failure
                        .downcast_ref::<&str>()
                        .map(|value| (*value).to_owned())
                })
                .ok_or("non-string iterator panic")?;
            Ok(json!({"buckets": null, "panic": message}))
        }
    }
}

fn result(input: &Value) -> Result<Value, Error> {
    let id = input["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("missing case ID")?;
    let mode = input["mode"].as_str().ok_or("missing kernel mode")?;
    let request = &input["request"];
    let borrowed = input["borrowed"]
        .as_bool()
        .ok_or("missing borrowed-mode flag")?;
    let actual = match mode {
        "float-iterator-set" => iterator_result(request, borrowed)?,
        "exponential-bound" => {
            let index = request["index"]
                .as_i64()
                .ok_or("invalid bucket index")?
                .try_into()?;
            let schema = request["schema"]
                .as_i64()
                .ok_or("invalid bucket schema")?
                .try_into()?;
            json!(format!(
                "{:016x}",
                bucket_bound(index, schema, None).to_bits()
            ))
        }
        "exponential-bound-table" => json!(
            (0..9)
                .map(|schema| {
                    let length = 1_i32 << schema;
                    (0..length)
                        .map(|fraction| {
                            format!(
                                "{:016x}",
                                bucket_bound(fraction - length, schema, None).to_bits()
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        ),
        "float-compact" => {
            let mut value = float(&request["input"])?;
            value.compact(
                request["max_empty"]
                    .as_i64()
                    .ok_or("invalid max_empty")?
                    .try_into()?,
            );
            encoded_float(&value)
        }
        "float-copy-to-schema" => {
            let value = float(&request["input"])?;
            let target = request["target_schema"]
                .as_i64()
                .ok_or("invalid target schema")?
                .try_into()?;
            let copied = if borrowed {
                value.as_ref().copy_to_schema(target)
            } else {
                value.copy_to_schema(target)
            };
            json!({"result": encoded_float(&copied), "input_after": encoded_float(&value)})
        }
        "float-reduce-resolution" => {
            let mut value = float(&request["input"])?;
            let target = request["target_schema"]
                .as_i64()
                .ok_or("invalid target schema")?
                .try_into()?;
            let error = value.reduce_resolution(target).err();
            json!({"result": encoded_float(&value), "error": error.as_ref().map_or_else(String::new, ToString::to_string),
                "histogram_error": error.as_ref().is_some_and(|error| error.is_data_error())})
        }
        "float-mul" | "float-div" => {
            let mut value = float(&request["input"])?;
            let factor = number(&request["scalar"])?;
            if mode == "float-mul" {
                value.mul(factor);
            } else {
                value.div(factor);
            }
            encoded_float(&value)
        }
        "float-copy" | "float-copy-to" => {
            let value = float(&request["input"])?;
            let mut destination = if request["destination"].is_null() {
                FloatHistogram::default()
            } else {
                float(&request["destination"])?
            };
            if mode == "float-copy" {
                destination = if borrowed {
                    value.as_ref().to_owned()
                } else {
                    value.copy()
                };
            } else if borrowed {
                value.as_ref().copy_to(&mut destination);
            } else {
                value.copy_to(&mut destination);
            }
            encoded_float(&destination)
        }
        "integer-copy" | "integer-copy-to" => {
            let value = integer(&request["input"])?;
            let mut destination = if request["destination"].is_null() {
                IntegerHistogram::default()
            } else {
                integer(&request["destination"])?
            };
            if mode == "integer-copy" {
                destination = if borrowed {
                    value.as_ref().to_owned()
                } else {
                    value.copy()
                };
            } else if borrowed {
                value.as_ref().copy_to(&mut destination);
            } else {
                value.copy_to(&mut destination);
            }
            encoded_integer(&destination)
        }
        "integer-to-float" => {
            let value = integer(&request["input"])?;
            let mut destination = if request["destination"].is_null() {
                FloatHistogram::default()
            } else {
                float(&request["destination"])?
            };
            if borrowed {
                value.as_ref().to_float_into(&mut destination);
            } else {
                value.to_float_into(&mut destination);
            }
            encoded_float(&destination)
        }
        "compensated-add" | "compensated-sub" => {
            let sum = number(&request["sum"])?;
            let increment = number(&request["increment"])?;
            let correction = number(&request["compensation"])?;
            let (sum, correction) = if mode == "compensated-add" {
                compensated_add(increment, sum, correction)
            } else {
                compensated_sub(increment, sum, correction)
            };
            json!({"sum": format!("{:016x}", sum.to_bits()), "compensation": format!("{:016x}", correction.to_bits())})
        }
        _ => return Err("unknown kernel mode".into()),
    };
    Ok(json!({"id": id, "actual": actual}))
}

fn main() -> Result<(), Error> {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line?)?;
        serde_json::to_writer(&mut output, &result(&input)?)?;
        writeln!(output)?;
    }
    Ok(())
}
