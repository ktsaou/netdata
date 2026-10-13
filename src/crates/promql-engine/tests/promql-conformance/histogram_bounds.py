#!/usr/bin/env python3
"""Generate immutable histogram bounds from a strictly verified pinned Go capture."""
import argparse
from pathlib import Path

from build import directory
from histogram_reference import COMMIT, load


def generate(capture):
    data, _ = load(capture)
    table = next(case["actual"] for case in data["cases"]
                 if case["id"] == "supplement:exponential-bound-table")
    lines = ["// SPDX-License-Identifier: GPL-3.0-or-later",
             "// Generated from Prometheus model/histogram/generic.go; see NOTICE.",
             f"// Source commit: {COMMIT}",
             "// Reproduce with tests/promql-conformance/histogram_bounds.py --capture <verified-capture>.",
             "#[rustfmt::skip]",
             "pub(super) const EXPONENTIAL_BOUNDS: [&[u64]; 9] = ["]
    for schema, values in enumerate(table):
        lines.extend([f"    // Schema {schema}.", "    &["])
        for offset in range(0, len(values), 4):
            lines.append("        " + ", ".join("0x" + value for value in values[offset:offset + 4]) + ",")
        lines.append("    ],")
    lines.append("];\n")
    root = directory(Path(__file__).resolve().parent.parent.parent)
    target = directory(root / "src") / "histogram_bounds_table.rs"
    target.write_text("\n".join(lines))
    print(f"generated {sum(map(len, table))} immutable bound constants: {target}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="verified histogram capture directory")
    args = parser.parse_args()
    generate(args.capture)
