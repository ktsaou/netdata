// SPDX-License-Identifier: GPL-3.0-or-later
//! Synthetic fixture for the actual typed C protocol primitives.
#[allow(dead_code)]
mod fixture_codec;
use fixture_codec::{Error, decoded_bytes, encoded_bytes};
use netdata_promql_engine::{
    c_abi::{self as c, types},
    query_time::Timestamp,
};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Write},
    ptr,
};

fn bytes(value: &[u8]) -> types::Bytes {
    types::Bytes {
        data: value.as_ptr(),
        len: value.len() as u64,
    }
}
unsafe fn borrowed<'a>(value: types::Bytes) -> &'a [u8] {
    if value.len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(value.data, value.len as usize) }
    }
}
unsafe fn evaluate(engine: *const types::Engine, request: &Value) -> Result<Value, Error> {
    let id = request["id"].as_str().ok_or("missing protocol id")?;
    let op = request["op"].as_str().ok_or("missing protocol operation")?;
    let mut result = json!({"id": id, "op": op});
    let mut report = ptr::null_mut();
    let status = match op {
        "time" | "time_boundary" => {
            let input = decoded_bytes(&request["input"])?;
            let mut time = types::Timestamp::default();
            let status = unsafe {
                c::netdata_promql_api_parse_time(
                    engine,
                    bytes(&input),
                    ptr::null(),
                    &mut time,
                    &mut report,
                )
            };
            if status == types::OK {
                let value = Timestamp {
                    unix_seconds: time.unix_seconds,
                    nanoseconds: time.nanoseconds,
                };
                result["timestamp"] = json!({"unix_seconds": value.unix_seconds, "nanoseconds": value.nanoseconds, "unix_ms": value.unix_milliseconds(), "evaluator_ms": value.evaluator_milliseconds()});
            }
            status
        }
        "duration" => {
            let input = decoded_bytes(&request["input"])?;
            let mut duration = 0;
            let status = unsafe {
                c::netdata_promql_api_parse_duration(
                    engine,
                    bytes(&input),
                    ptr::null(),
                    &mut duration,
                    &mut report,
                )
            };
            if status == types::OK {
                result["duration_ns"] = json!(duration);
            }
            status
        }
        "sample_float" | "scalar_float" => {
            let bits =
                u64::from_str_radix(request["bits"].as_str().ok_or("missing float bits")?, 16)?;
            let mut text = types::NumericText::default();
            let status = if op == "sample_float" {
                unsafe { c::netdata_promql_api_float_text(bits, &mut text) }
            } else {
                unsafe { c::netdata_promql_api_scalar_float_text(bits, &mut text) }
            };
            result["text"] = json!(encoded_bytes(&text.bytes[..text.len as usize]));
            let text = std::str::from_utf8(&text.bytes[..text.len as usize])?;
            result["raw_json"] = json!(encoded_bytes(
                if op == "sample_float" {
                    format!("\"{text}\"")
                } else {
                    format!("[0,\"{text}\"]")
                }
                .as_bytes()
            ));
            status
        }
        "sample_timestamp" | "scalar_timestamp" => {
            let time = request["time_ms"]
                .as_str()
                .ok_or("missing timestamp")?
                .parse()?;
            let mut text = types::NumericText::default();
            let status = if op == "sample_timestamp" {
                unsafe { c::netdata_promql_api_timestamp_text(time, &mut text) }
            } else {
                unsafe { c::netdata_promql_api_scalar_timestamp_text(time, &mut text) }
            };
            let text = std::str::from_utf8(&text.bytes[..text.len as usize])?;
            result["raw_json"] = json!(encoded_bytes(
                if op == "sample_timestamp" {
                    text.to_owned()
                } else {
                    format!("[{text},\"1\"]")
                }
                .as_bytes()
            ));
            status
        }
        "selector" => {
            let input = decoded_bytes(&request["input"])?;
            let mut selector = ptr::null_mut();
            let status = unsafe {
                c::netdata_promql_selector_compile(
                    engine,
                    bytes(&input),
                    ptr::null(),
                    &mut selector,
                    &mut report,
                )
            };
            if status == types::OK {
                let mut count = 0;
                assert_eq!(
                    unsafe { c::netdata_promql_selector_matcher_count(selector, &mut count) },
                    types::OK
                );
                let mut matchers = Vec::new();
                for index in 0..count {
                    let mut matcher = types::Matcher::default();
                    assert_eq!(
                        unsafe {
                            c::netdata_promql_selector_matcher(selector, index, &mut matcher)
                        },
                        types::OK
                    );
                    matchers.push(json!({"kind": matcher.kind, "name": encoded_bytes(unsafe { borrowed(matcher.name) }), "value": encoded_bytes(unsafe { borrowed(matcher.value) })}));
                }
                let mut nonempty = 0;
                assert_eq!(
                    unsafe {
                        c::netdata_promql_selector_nonempty(selector, ptr::null(), &mut nonempty)
                    },
                    types::OK
                );
                result["matchers"] = json!(matchers);
                result["nonempty"] = json!(nonempty != 0);
                if let Some(sets) = request["labelsets"].as_array() {
                    let mut matches = Vec::new();
                    for set in sets {
                        let inputs = set["labels"].as_array().ok_or("missing probe labels")?;
                        let buffers = inputs
                            .iter()
                            .map(|label| {
                                Ok((
                                    decoded_bytes(&label["name"])?,
                                    decoded_bytes(&label["value"])?,
                                ))
                            })
                            .collect::<Result<Vec<_>, Error>>()?;
                        let labels = buffers
                            .iter()
                            .map(|(name, value)| types::Label {
                                name: bytes(name),
                                value: bytes(value),
                            })
                            .collect::<Vec<_>>();
                        let mut matched = 0;
                        let status = unsafe {
                            c::netdata_promql_selector_matches(
                                selector,
                                types::Labels {
                                    data: labels.as_ptr(),
                                    len: labels.len() as u64,
                                },
                                ptr::null(),
                                &mut matched,
                            )
                        };
                        if status != types::OK {
                            return Err(
                                format!("invalid selector probe label set: {status}").into()
                            );
                        }
                        matches.push(json!({"id": set["id"], "matches": matched != 0}));
                    }
                    result["labelset_matches"] = json!(matches);
                }
                assert_eq!(
                    unsafe { c::netdata_promql_selector_release(selector) },
                    types::OK
                );
            }
            status
        }
        _ => return Err("unknown protocol fixture operation".into()),
    };
    result["success"] = json!(status == types::OK);
    if !report.is_null() {
        if status != types::OK {
            let mut count = 0;
            assert_eq!(
                unsafe { c::netdata_promql_report_error_count(report, &mut count) },
                types::OK
            );
            let mut errors = Vec::new();
            for index in 0..count {
                let mut error = types::Error::default();
                assert_eq!(
                    unsafe { c::netdata_promql_report_error(report, index, &mut error) },
                    types::OK
                );
                errors.push(json!({"message": encoded_bytes(unsafe { borrowed(error.message) }), "start": error.position.start, "end": error.position.end, "position_present": error.position_present}));
            }
            result["errors"] = json!(errors);
            result["error_message"] = result["errors"][0]["message"].clone();
        }
        assert_eq!(
            unsafe { c::netdata_promql_report_release(report) },
            types::OK
        );
    }
    Ok(result)
}
fn main() -> Result<(), Error> {
    let mut engine = ptr::null_mut();
    assert_eq!(
        unsafe { c::netdata_promql_engine_create(ptr::null(), &mut engine) },
        types::OK
    );
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line?)?;
        let response = unsafe { evaluate(engine, &request) }?;
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
    }
    assert_eq!(
        unsafe { c::netdata_promql_engine_release(engine) },
        types::OK
    );
    output.flush()?;
    Ok(())
}
