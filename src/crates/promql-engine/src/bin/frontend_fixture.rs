// SPDX-License-Identifier: GPL-3.0-or-later
//! Test transport: IDs, mode, token-count guard and hex expression, separated by tabs.
use std::io::{self, BufRead, Write};

use netdata_promql_engine::frontend::lexer::Lexer;
use netdata_promql_engine::frontend::literal::unquote;

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
        if fields.len() != 4 || fields[0].is_empty() || !matches!(fields[1], "q" | "s" | "u") {
            return Err("expected ID, query/series mode, fixture token guard and hex input".into());
        }
        let count: usize = fields[2].parse()?;
        let input = decode(fields[3])?;
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
