#!/usr/bin/env python3
"""Qualify literal helpers against independently captured pinned Go results."""
import base64
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


def qualify():
    reference = json.loads((HERE / "literal-cases.json").read_text())
    if reference["commit"] != "5241a27fe3c6983549fccc32f6e65917408c63cd":
        raise ValueError("literal reference pin changed")
    if reference["architecture"] != "amd64" or reference["go_version"] != "go1.27.1":
        raise ValueError("literal conversion reference targets Go 1.27.1 / amd64")
    requests, expected = [], {}
    for case in reference["cases"]:
        ident = case["id"]
        requests.append(f'{ident}\t{"d" if case["duration"] else "n"}\t1\t'
                        + base64.b64decode(case["input"], validate=True).hex())
        expected[ident] = [("NUMBER_BITS", "0", case["value_bits"]),
                           ("NANOSECONDS", "0", str(case["nanoseconds"]))]
        expected[ident] += [("ERROR", "0", base64.b64decode(error, validate=True).hex())
                            for error in case["errors"]]
    output = subprocess.check_output([HERE / ".tmp/core-target/release/promql-frontend-fixture"],
                                    input=("\n".join(requests)+"\n").encode()).decode()
    actual, ended = {}, set()
    for line in output.splitlines():
        fields = line.split("\t")
        ident = fields[0]
        if ident not in expected or ident in ended:
            raise ValueError("extra literal candidate output")
        if fields[1:] == ["END"]:
            ended.add(ident)
        elif len(fields) == 4:
            actual.setdefault(ident, []).append(tuple(fields[1:]))
        else:
            raise ValueError("malformed literal candidate output")
    if ended != set(expected):
        raise ValueError("missing literal candidate case")
    failed = [{"id": ident, "expected": items, "actual": actual.get(ident)}
              for ident, items in expected.items() if actual.get(ident) != items]
    print(json.dumps({"literal_cases": len(expected), "passed": len(expected)-len(failed),
                      "architecture": reference["architecture"], "failed": failed}))
    if failed:
        raise SystemExit(1)


if __name__ == "__main__":
    qualify()
