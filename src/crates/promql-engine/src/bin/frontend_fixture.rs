// SPDX-License-Identifier: GPL-3.0-or-later
//! Test transport: IDs, mode, token-count guard and hex expression, separated by tabs.
use std::io::{self, BufRead, Write};

use netdata_promql_engine::frontend::lexer::Lexer;
use netdata_promql_engine::frontend::literal::unquote;
use netdata_promql_engine::frontend::number;
use netdata_promql_engine::frontend::{ast::ParserOptions, parser, regex::Selector};
use netdata_promql_engine::numerical::{FloatTextStyle, float_text};

#[path = "frontend_fixture_output.rs"]
mod output;
mod regex_fixture;

fn decode(text: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if !text.is_ascii() || !text.len().is_multiple_of(2) {
        return Err("invalid hex expression".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&text[i..i + 2], 16)?))
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 4
            || fields[0].is_empty()
            || !matches!(
                fields[1],
                "q" | "s" | "u" | "n" | "d" | "p" | "r" | "f" | "g" | "a"
            )
        {
            return Err("expected ID, query/series mode, fixture token guard and hex input".into());
        }
        let count: usize = fields[2].parse()?;
        let input = decode(fields[3])?;
        if fields[1] == "a" {
            if count != 0 {
                return Err("invalid annotation fixture projection".into());
            }
            let result = netdata_promql_engine::annotations::fixture::result(fields[0], &input)?;
            serde_json::to_writer(&mut out, &result)?;
            writeln!(out)?;
            continue;
        }
        if fields[1] == "g" {
            let bytes: [u8; 8] = input.try_into().map_err(|_| "expected binary64 bytes")?;
            let style = match count {
                0 => FloatTextStyle::Shortest,
                1 => FloatTextStyle::TwoSignificant,
                2 => FloatTextStyle::OpenMetrics,
                _ => return Err("invalid float formatting projection".into()),
            };
            let text = float_text(f64::from_bits(u64::from_be_bytes(bytes)), style);
            let (roundtrip, error) = number::float_fixture(text.as_bytes());
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({
                    "id": fields[0], "text": text.as_str(),
                    "roundtrip_bits": format!("{:016x}", roundtrip.to_bits()), "roundtrip_error": error,
                }),
            )?;
            writeln!(out)?;
            continue;
        }
        if fields[1] == "f" {
            let (value, error) = number::float_fixture(&input);
            serde_json::to_writer(
                &mut out,
                &serde_json::json!({
                    "id": fields[0], "value_bits": format!("{:016x}", value.to_bits()), "error": error,
                }),
            )?;
            writeln!(out)?;
            continue;
        }
        if fields[1] == "r" {
            serde_json::to_writer(&mut out, &regex_fixture::result(fields[0], &input)?)?;
            writeln!(out)?;
            continue;
        }
        if fields[1] == "p" {
            let options = ParserOptions {
                experimental_functions: count & 1 != 0,
                binop_fill_modifiers: count & 2 != 0,
                extended_range_selectors: count & 4 != 0,
                duration_expressions: count & 8 != 0,
            };
            let parsed = parser::parse(&input, options, Selector::compile);
            serde_json::to_writer(&mut out, &output::result(fields[0], parsed))?;
            writeln!(out)?;
            continue;
        }
        if matches!(fields[1], "n" | "d") {
            let parsed = if fields[1] == "n" {
                number::number(&input)
            } else {
                match number::duration(&input) {
                    Ok(nanoseconds) => number::Number {
                        value: number::seconds(nanoseconds),
                        error: None,
                    },
                    Err(error) => number::Number {
                        value: 0.0,
                        error: Some(error),
                    },
                }
            };
            writeln!(
                out,
                "{}\tNUMBER_BITS\t0\t{:016x}",
                fields[0],
                parsed.value.to_bits()
            )?;
            writeln!(
                out,
                "{}\tNANOSECONDS\t0\t{}",
                fields[0],
                number::nanoseconds(parsed.value)
            )?;
            if let Some(error) = parsed.error {
                write!(out, "{}\tERROR\t0\t", fields[0])?;
                for byte in error {
                    write!(out, "{byte:02x}")?;
                }
                writeln!(out)?;
            }
            writeln!(out, "{}\tEND", fields[0])?;
            continue;
        }
        if fields[1] == "u" {
            let value = unquote(&input)?;
            write!(out, "{}\tSTRING_VALUE\t0\t", fields[0])?;
            for byte in value.iter() {
                write!(out, "{byte:02x}")?;
            }
            writeln!(out)?;
            writeln!(out, "{}\tEND", fields[0])?;
            continue;
        }
        // This guard belongs only to fixture transport, not to engine resource enforcement.
        for item in Lexer::with_series_description(&input, fields[1] == "s").take(count) {
            write!(
                out,
                "{}\t{}\t{}\t",
                fields[0],
                item.kind.name(),
                item.span.start
            )?;
            for byte in item.value.iter() {
                write!(out, "{byte:02x}")?;
            }
            writeln!(out)?;
        }
        writeln!(out, "{}\tEND", fields[0])?;
    }
    out.flush()?;
    Ok(())
}
