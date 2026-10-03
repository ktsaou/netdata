#!/usr/bin/env python3
"""Run the common profile and focused probes against the independent Go oracle."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess

from compare import compare, difference, numeric_difference_report

HERE = Path(__file__).resolve().parent

def load(path):
    return json.loads(path.read_text())

def invoke(command, output=None, env=None):
    print(shlex.join(map(str, command)), flush=True)
    result = subprocess.run(list(map(str, command)), env=env, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"exit {result.returncode}: {shlex.join(map(str, command))}\n{result.stderr[-8000:]}")
    if result.stderr:
        print(result.stderr[-2000:], end="")
    if output:
        output.write_text(result.stdout)
        return json.loads(result.stdout)
    return result.stdout.strip()

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--dataset", type=Path, default=HERE / "dataset.json")
    parser.add_argument("--cases", type=Path, default=HERE / "cases.json")
    parser.add_argument("--baseline-only", action="store_true")
    args = parser.parse_args()
    mode = "sanitize" if args.sanitize else "release"
    output = HERE / "build" / mode / ("replay" if args.baseline_only else "results")
    output.mkdir(parents=True, exist_ok=True)
    oracle = HERE / "oracle/.cache/oracle"
    cases = load(args.cases)
    reference = invoke([oracle, args.dataset, args.cases], output / "reference.json")
    # Independent fixture arithmetic catches an oracle accidentally returning empty data.
    if not args.baseline_only:
        ref = {r["id"]: r for r in reference["results"]}
        assert ref["B001"]["rows"][0]["points"] == [[600000, "2"]]
        assert ref["B036"]["rows"][0]["points"] == [[600000, "175"]]
        assert ref["B045"]["rows"][0]["points"] == [[600000, "5"]]
        assert ref["B067"]["kind"] == 3 and [p[0] for p in ref["B067"]["rows"][0]["points"]] == [480000, 540000, 600000]
    summary = {"dataset_sha256": hashlib.sha256(args.dataset.read_bytes()).hexdigest(),
               "cases_sha256": hashlib.sha256(args.cases.read_bytes()).hexdigest(), "mode": mode, "candidates": {},
               "oracle_annotations": {r["id"]:{"warnings":r.get("warnings",[]),"infos":r.get("infos",[])} for r in reference["results"] if r.get("warnings") or r.get("infos")}}
    environment = os.environ.copy()
    if args.sanitize:
        environment.update(ASAN_OPTIONS="detect_leaks=1:abort_on_error=1", UBSAN_OPTIONS="halt_on_error=1:print_stacktrace=1")
    regression_reference = risk_reference = None
    if not args.baseline_only:
        regression_reference = invoke([oracle, args.dataset, HERE / "regression.json"], output / "regression-reference.json")
        risk_reference = invoke([oracle, args.dataset, HERE / "risk.json"], output / "risk-reference.json")
    failed = False
    for language in ["c", "cpp", "rust"]:
        host = HERE / "build" / mode / f"{language}-host"
        got = invoke([host, args.dataset, args.cases], output / f"{language}-baseline.json", environment)
        failures = compare(reference, got, cases)
        evidence = {"profile_passed": len(cases)-len(failures), "profile_total": len(cases), "failures": failures}
        evidence["numeric_bits"] = {"profile": numeric_difference_report(reference, got, cases)}
        if not args.baseline_only:
            evidence["ownership"] = invoke([HERE / "build" / mode / f"{language}-ownership"], env=environment)
            regression = load(HERE / "regression.json")
            got = invoke([host, args.dataset, HERE / "regression.json"], output / f"{language}-regression.json", environment)
            evidence["regression_failures"] = compare(regression_reference, got, regression)
            evidence["regression_passed"] = len(regression)-len(evidence["regression_failures"])
            evidence["numeric_bits"]["regressions"] = numeric_difference_report(regression_reference, got, regression)
            got = invoke([host, args.dataset, HERE / "risk.json"], output / f"{language}-risk.json", environment)
            refs = {r["id"]: r for r in risk_reference["results"]}
            evidence["risk_failures"] = {}
            for r in got["results"]:
                ident = r["id"]
                if ident in ["R068", "R069", "R-after"]:
                    err = difference(refs[ident], r)
                else:
                    needles = {"R-cancel": "cancelled", "R-budget": "budget", "R-fault": "allocation", "R-invalid": "expected", "R-deep": "depth limit", "R-long": "token limit", "R-unwind": "panic contained" if language == "rust" else "evaluation"}
                    err = None if r["kind"] == 4 and needles[ident] in r.get("error", "") else "expected controlled failure"
                    if ident == "R-cancel" and r["work"] != 30:
                        err = "cancellation must happen after temporary rows exist"
                if err:
                    evidence["risk_failures"][ident] = err
        summary["candidates"][language] = evidence
        failed |= bool(failures or evidence.get("regression_failures") or evidence.get("risk_failures"))
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    raise SystemExit(bool(failed))

if __name__ == "__main__":
    main()
