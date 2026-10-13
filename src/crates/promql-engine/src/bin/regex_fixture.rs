// SPDX-License-Identifier: GPL-3.0-or-later
//! Byte-preserving transport for independently captured regex qualification.
use std::ops::Range;

use netdata_promql_engine::frontend::regex::{self, GoRegex, Selector};
use serde_json::{Value, json};

use super::{decode, output::bytes};

fn byte_list(value: &Value) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error>> {
    value
        .as_array()
        .ok_or("expected byte list")?
        .iter()
        .map(|value| decode(value.as_str().ok_or("expected hex bytes")?))
        .collect()
}

fn offsets(captures: &[Option<Range<usize>>]) -> Vec<i64> {
    captures
        .iter()
        .flat_map(|range| {
            range
                .as_ref()
                .map_or([-1, -1], |range| [range.start as i64, range.end as i64])
        })
        .collect()
}

pub fn result(id: &str, input: &[u8]) -> Result<Value, Box<dyn std::error::Error>> {
    let request: Value = serde_json::from_slice(input)?;
    let pattern = decode(request["pattern"].as_str().ok_or("missing regex pattern")?)?;
    let mode = request["mode"].as_str().ok_or("missing regex mode")?;
    let mut expected = json!({"error": null});
    if mode == "program" {
        match regex::inspect_program(&pattern) {
            Err(error) => expected["error"] = json!(bytes(&error)),
            Ok(program) => {
                expected["start"] = json!(program.start);
                expected["num_cap"] = json!(program.num_cap);
                expected["one_pass"] = json!(program.one_pass);
                expected["prefix"] = json!(bytes(&program.prefix));
                expected["prefix_end"] = json!(program.prefix_end);
                expected["prefix_complete"] = json!(program.complete);
                expected["instructions"] = json!(
                    program
                        .instructions
                        .iter()
                        .map(|i| {
                            json!({"op": i.code.name(), "out": i.out, "arg": i.arg,
                        "runes": i.rune_field()})
                        })
                        .collect::<Vec<_>>()
                );
            }
        }
    } else if mode == "normalize" {
        let subject = decode(
            request["subject"]
                .as_str()
                .ok_or("missing normalization subject")?,
        )?;
        let (normalized, lower) = regex::inspect_normalization(&subject);
        expected["normalized"] = json!(bytes(&normalized));
        expected["lower"] = json!(bytes(&lower));
    } else if matches!(mode, "syntax" | "standard-syntax") {
        let flags = u16::try_from(request["flags"].as_u64().ok_or("missing syntax flags")?)?;
        match regex::inspect_syntax(&pattern, flags, mode == "standard-syntax") {
            Ok(inspection) => {
                expected["names"] = json!(
                    inspection
                        .names
                        .iter()
                        .map(|name| bytes(name))
                        .collect::<Vec<_>>()
                );
                expected["canonical"] = json!(bytes(&inspection.canonical));
            }
            Err(error) => {
                expected["error"] = json!({"code": bytes(error.code.as_bytes()),
                "expression": bytes(&error.expression), "message": bytes(&error.bytes())});
            }
        }
    } else {
        let subject = decode(request["subject"].as_str().ok_or("missing regex subject")?)?;
        if mode == "selector" {
            match Selector::compile(&pattern) {
                Ok(selector) => {
                    expected["matches"] = json!(selector.matches(&subject));
                }
                Err(error) => {
                    expected["error"] = json!(bytes(&error));
                }
            }
        } else {
            let compiled = if mode == "label_replace" {
                GoRegex::compile_label_replace(&pattern)
            } else {
                GoRegex::compile(&pattern)
            };
            match compiled {
                Err(error) => {
                    expected["error"] = json!(bytes(&error));
                }
                Ok(regex) => {
                    expected["num_subexp"] = json!(regex.names().len() - 1);
                    expected["names"] = json!(
                        regex
                            .names()
                            .iter()
                            .map(|name| bytes(name))
                            .collect::<Vec<_>>()
                    );
                    let lookups = byte_list(&request["lookups"])?;
                    expected["indices"] = json!(
                        lookups
                            .iter()
                            .map(|name| regex.subexp_index(name).map_or(-1, |index| index as i64))
                            .collect::<Vec<_>>()
                    );
                    if matches!(mode, "find" | "label_replace") {
                        let first = regex.captures(&subject);
                        expected["match_string"] = json!(first.is_some());
                        expected["first"] = json!(first.map(|captures| offsets(&captures)));
                        let captures = regex.captures_all(&subject);
                        expected["matches"] = if captures.is_empty() {
                            Value::Null
                        } else {
                            json!(
                                captures
                                    .iter()
                                    .map(|captures| offsets(captures))
                                    .collect::<Vec<_>>()
                            )
                        };
                        let templates = byte_list(&request["templates"])?;
                        expected["expansions"] = json!(
                            templates
                                .iter()
                                .map(|template| captures
                                    .iter()
                                    .map(|captures| bytes(
                                        &regex.expand(template, &subject, captures)
                                    ))
                                    .collect::<Vec<_>>())
                                .collect::<Vec<_>>()
                        );
                        expected["replacements"] = json!(
                            templates
                                .iter()
                                .map(|template| {
                                    let mut output = Vec::new();
                                    let mut previous_end = 0;
                                    for captures in &captures {
                                        let whole = captures[0].as_ref().unwrap();
                                        output
                                            .extend_from_slice(&subject[previous_end..whole.start]);
                                        output.extend(regex.expand(template, &subject, captures));
                                        previous_end = whole.end;
                                    }
                                    output.extend_from_slice(&subject[previous_end..]);
                                    bytes(&output)
                                })
                                .collect::<Vec<_>>()
                        );
                    } else if mode != "compile" {
                        return Err("unknown regex mode".into());
                    }
                }
            }
        }
    }
    Ok(json!({"id": id, "expected": expected}))
}
