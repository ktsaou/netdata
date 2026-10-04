#!/usr/bin/env python3
"""Compare every captured frontend case against the owned Rust implementation."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess

from build import directory
from capture import validate_frontend

HERE = Path(__file__).resolve().parent


def qualify(capture_directory):
    capture_directory = directory(capture_directory)
    manifest = json.loads((capture_directory / "manifest.json").read_text())
    raw = (capture_directory / "frontend.json").read_bytes()
    digest = manifest.get("outputs", {}).get("frontend", {}).get("sha256", manifest.get("frontend_sha256"))
    if hashlib.sha256(raw).hexdigest() != digest:
        raise ValueError("captured frontend digest changed")
    frontend = json.loads(raw)
    validate_frontend(frontend)
    if frontend.get("go_version") != "go1.27.1" or frontend.get("unicode_version") != "17.0.0":
        raise ValueError("Rust diagnostic tables target Go 1.27.1 / Unicode 17.0.0")
    cases = frontend["lexers"]
    requests = [f'{case["id"]}\t{"s" if case["series_description"] else "q"}\t{len(case["tokens"])+1}\t'
                + base64.b64decode(case["input"], validate=True).hex() for case in cases]
    literals = {}
    for case in frontend["expressions"]:
        if case["declared_failure"]:
            continue
        expression = base64.b64decode(case["input"], validate=True)
        for index, node in enumerate(case["ast"]["nodes"]):
            if node["node"] == "StringLiteral":
                ident = f'{case["id"]}:string:{index}'
                fields = node["fields"]
                span = fields["PosRange"]
                literal = expression[span["Start"]:span["End"]]
                literals[ident] = [{"type": "STRING_VALUE", "position": 0,
                                   "value": base64.b64decode(fields["Val"], validate=True)}]
                requests.append(f"{ident}\tu\t1\t{literal.hex()}")
    candidate = HERE / ".tmp/core-target/release/promql-frontend-fixture"
    output = subprocess.check_output([candidate], input=("\n".join(requests)+"\n").encode()).decode()
    actual, ended = {}, set()
    for line in output.splitlines():
        fields = line.split("\t")
        ident = fields[0]
        if ident in ended:
            raise ValueError("candidate emitted output after a case ended")
        if fields[1] == "END" and len(fields) == 2:
            ended.add(ident)
            continue
        if len(fields) != 4:
            raise ValueError("malformed candidate lexer item")
        actual.setdefault(ident, []).append({"type": fields[1], "position": int(fields[2]), "value": bytes.fromhex(fields[3])})
    if ended != {case["id"] for case in cases} | set(literals) or set(actual) != ended:
        raise ValueError("missing or extra candidate lexer case")
    failed = []
    for case in cases:
        expected = [{"type": frontend["token_catalog"][str(token["type"])], "position": token["position"],
                     "value": base64.b64decode(token["value"], validate=True)} for token in case["tokens"]]
        if actual[case["id"]] != expected:
            failed.append({"id": case["id"], "expected": expected, "actual": actual[case["id"]]})
    lexer_failures = len(failed)
    for ident, expected in literals.items():
        if actual[ident] != expected:
            failed.append({"id": ident, "expected": expected, "actual": actual[ident]})
    report = {"commit": frontend["commit"], "lexer_cases": len(cases), "lexer_passed": len(cases)-lexer_failures,
              "string_literal_cases": len(literals), "string_literals_passed": len(literals)-(len(failed)-lexer_failures),
              "parser_cases": 356, "parser_status": "Not implemented or executed.", "failed": failed}
    print(json.dumps(report, default=lambda b: {"bytes_hex": b.hex()}))
    if failed:
        raise SystemExit(1)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="existing captured reference directory")
    args = parser.parse_args()
    qualify(args.capture)
