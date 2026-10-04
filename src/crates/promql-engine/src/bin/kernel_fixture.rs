// SPDX-License-Identifier: GPL-3.0-or-later
//! Test-only per-sample kernel transport; it is not a storage/evaluation adapter.
use std::io::{self, BufRead, Write};

use netdata_promql_engine::histogram::{
    Bucket, FloatHistogram, HistogramArithmetic, HistogramError, IntegerHistogram, Span,
    bucket_bound, custom_bounds_match,
};
use netdata_promql_engine::numerical::{compensated_add, compensated_sub, frexp, log, log2, sqrt};
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

fn encoded_bucket(bucket: Bucket) -> Value {
    json!({"lower": format!("{:016x}", bucket.lower.to_bits()),
        "upper": format!("{:016x}", bucket.upper.to_bits()),
        "count": format!("{:016x}", bucket.count.to_bits()), "index": bucket.index,
        "lower_inclusive": bucket.lower_inclusive, "upper_inclusive": bucket.upper_inclusive})
}

fn encoded_iterator(iterator: impl Iterator<Item = Bucket>) -> Value {
    let buckets: Vec<_> = iterator.map(encoded_bucket).collect();
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

fn arithmetic_checkpoint(
    result: Option<&FloatHistogram>,
    receiver: &FloatHistogram,
    argument: &FloatHistogram,
    correction: Option<&FloatHistogram>,
    outcome: &Result<HistogramArithmetic, HistogramError>,
) -> Value {
    let flags = outcome.as_ref().copied().unwrap_or_default();
    json!({"result": result.map(encoded_float), "receiver_after": encoded_float(receiver),
        "argument_after": encoded_float(argument), "compensation_after": correction.map(encoded_float),
        "counter_reset_collision": flags.counter_reset_collision,
        "nhcb_bounds_reconciled": flags.nhcb_bounds_reconciled,
        "error": outcome.as_ref().err().map_or_else(String::new, ToString::to_string),
        "histogram_error": outcome.as_ref().err().is_some_and(|error| error.is_data_error())})
}

fn arithmetic_result(operation: &Value) -> Result<Value, Error> {
    let mut left = float(&operation["left"])?;
    let right = float(&operation["right"])?;
    let supplied = !operation["compensation"].is_null();
    let mut correction = if supplied {
        Some(float(&operation["compensation"])?)
    } else {
        None
    };
    let kind = operation["operation"]
        .as_str()
        .ok_or("invalid arithmetic operation")?;
    let outcome = match kind {
        "add" => left.add(right.as_ref()),
        "sub" => left.sub(right.as_ref()),
        "kahan" => left.kahan_add(right.as_ref(), &mut correction),
        _ => return Err("unknown arithmetic operation".into()),
    };
    let result = if outcome.is_err() {
        None
    } else if kind == "kahan" {
        correction.as_ref()
    } else {
        Some(&left)
    };
    let raw = arithmetic_checkpoint(
        result,
        &left,
        &right,
        if supplied { correction.as_ref() } else { None },
        &outcome,
    );
    let mut fold = Value::Null;
    let mut succeeded = outcome.is_ok();
    if kind == "kahan" && outcome.is_ok() {
        let outcome = left.add(correction.as_ref().ok_or("missing correction")?.as_ref());
        succeeded = outcome.is_ok();
        fold = arithmetic_checkpoint(
            outcome.is_ok().then_some(&left),
            &left,
            &right,
            None,
            &outcome,
        );
    }
    let mut final_value = Value::Null;
    if operation["compact"]
        .as_bool()
        .ok_or("invalid compaction flag")?
        && succeeded
    {
        left.compact(0);
        final_value = encoded_float(&left);
    }
    Ok(json!({"raw": raw, "fold": fold, "final": final_value}))
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
        "math-frexp" => {
            let (fraction, exponent) = frexp(number(&request["input"])?);
            json!({"fraction": format!("{:016x}", fraction.to_bits()), "exponent": exponent})
        }
        "math-log" | "math-log2" | "math-sqrt" => {
            let value = number(&request["input"])?;
            let result = match mode {
                "math-log" => log(value),
                "math-log2" => log2(value),
                _ => sqrt(value),
            };
            json!(format!("{:016x}", result.to_bits()))
        }
        "math-set" => {
            let value = number(&request["input"])?;
            let (fraction, exponent) = frexp(value);
            json!({"frexp": {"fraction": format!("{:016x}", fraction.to_bits()), "exponent": exponent},
                "log": format!("{:016x}", log(value).to_bits()),
                "log2": format!("{:016x}", log2(value).to_bits()),
                "sqrt": format!("{:016x}", sqrt(value).to_bits())})
        }
        "float-equality-pair" => {
            let left = float(&request["left"])?;
            let right = float(&request["right"])?;
            let (forward, reverse) = if borrowed {
                (
                    left.as_ref().equals(right.as_ref()),
                    right.as_ref().equals(left.as_ref()),
                )
            } else {
                (left.equals(right.as_ref()), right.equals(left.as_ref()))
            };
            json!({"forward": forward, "reverse": reverse,
                "left_after": encoded_float(&left), "right_after": encoded_float(&right)})
        }
        "float-overflow" => {
            let value = float(&request["input"])?;
            let overflow = if borrowed {
                value.as_ref().has_overflow()
            } else {
                value.has_overflow()
            };
            json!({"overflow": overflow, "input_after": encoded_float(&value)})
        }
        "float-arithmetic-set" => json!({"operations": request["operations"].as_array()
            .ok_or("invalid arithmetic inventory")?.iter().map(arithmetic_result).collect::<Result<Vec<_>, _>>()?}),
        "float-kahan-compact" => {
            let mut value = float(&request["input"])?;
            let mut correction = float(&request["compensation"])?;
            value.kahan_compact(
                request["max_empty"]
                    .as_i64()
                    .ok_or("invalid max_empty")?
                    .try_into()?,
                &mut correction,
            );
            json!({"input_after": encoded_float(&value), "compensation_after": encoded_float(&correction)})
        }
        "custom-bounds-match" => {
            let left = array(&request["left"], number)?;
            let right = array(&request["right"], number)?;
            json!({"forward": custom_bounds_match(left.as_deref(), right.as_deref()),
                "reverse": custom_bounds_match(right.as_deref(), left.as_deref())})
        }
        "float-detect-reset" => {
            let current = float(&request["current"])?;
            let previous = float(&request["previous"])?;
            let reset = if borrowed {
                current.as_ref().detect_reset(previous.as_ref())
            } else {
                current.detect_reset(previous.as_ref())
            };
            json!({"reset": reset, "current_after": encoded_float(&current), "previous_after": encoded_float(&previous)})
        }
        "float-zero-threshold" => {
            let value = float(&request["input"])?;
            let correction = if request["compensation"].is_null() {
                None
            } else {
                Some(float(&request["compensation"])?)
            };
            let correction_ref = correction.as_ref().map(FloatHistogram::as_ref);
            let threshold = number(&request["threshold"])?;
            let (result, panic) =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if borrowed {
                        value
                            .as_ref()
                            .zero_count_for_threshold(threshold, correction_ref)
                    } else {
                        value.zero_count_for_threshold(threshold, correction_ref)
                    }
                })) {
                    Ok(result) => (
                        json!({"count": format!("{:016x}", result.count.to_bits()),
                    "threshold": format!("{:016x}", result.threshold.to_bits()),
                    "compensation": format!("{:016x}", result.compensation.to_bits())}),
                        String::new(),
                    ),
                    Err(failure) => {
                        let message = failure
                            .downcast_ref::<String>()
                            .cloned()
                            .or_else(|| {
                                failure
                                    .downcast_ref::<&str>()
                                    .map(|value| (*value).to_owned())
                            })
                            .ok_or("non-string threshold panic")?;
                        (Value::Null, message)
                    }
                };
            json!({"result": result, "panic": panic, "input_after": encoded_float(&value),
                "compensation_after": correction.as_ref().map(encoded_float)})
        }
        "float-zero-bucket" => {
            let value = float(&request["input"])?;
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if borrowed {
                    value.as_ref().zero_bucket()
                } else {
                    value.zero_bucket()
                }
            })) {
                Ok(bucket) => json!({"bucket": encoded_bucket(bucket), "panic": ""}),
                Err(failure) => {
                    let message = failure
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| {
                            failure
                                .downcast_ref::<&str>()
                                .map(|value| (*value).to_owned())
                        })
                        .ok_or("non-string zero-bucket panic")?;
                    json!({"bucket": null, "panic": message})
                }
            }
        }
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
