"""Capture complete original cursor outcomes and qualify paged candidate replay."""

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
from copy import deepcopy
from pathlib import Path

from build import directory
from capture import verify_facade
from compare import checked_results, compare
from histogram_reference import COMMIT, MODULE, input_ledger
from query import checked_candidate_packets, dataset_hex, request_hex

HERE = Path(__file__).resolve().parent
INPUT_SHA256 = "8e571234c7f841b4c89059dc2934981869968bb87bb1017cc0d1f53444b9186d"
PROFILE = {
    "schema_version": 1,
    "commit": COMMIT,
    "reference": MODULE,
    "go_version": "go1.27.1",
    "go_os": "linux",
    "go_arch": "amd64",
    "unicode_version": "17.0.0",
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def manifest_digest(value):
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def cases():
    path = HERE / "cursor-original-inputs.json"
    if sha(path) != INPUT_SHA256:
        raise ValueError("independently declared cursor inputs changed")
    result = json.loads(path.read_bytes())
    if (
        type(result) is not list
        or len(result) != 82
        or len({row["request"]["id"] for row in result}) != len(result)
    ):
        raise ValueError("cursor input roster changed")
    return result


def helper_files(construction_errors=False):
    # Reuse the canonical codec and engine options without changing its entry point.
    files = {
        f"helper/{path.relative_to(HERE / 'oracle').as_posix()}": path.read_bytes()
        for path in sorted((HERE / "oracle").rglob("*.go"))
        if not path.name.endswith("_test.go")
    }
    for name in ("go.mod", "go.sum"):
        files[f"helper/{name}"] = (HERE / "oracle" / name).read_bytes()
    main = files["helper/main.go"]
    if main.count(b"func main() {") != 1:
        raise ValueError("canonical oracle entry point changed")
    files["helper/main.go"] = main.replace(b"func main() {", b"func oracleMain() {")
    files["helper/cursor_phase.go"] = (HERE / "capture/cursor_phase.go").read_bytes()
    if construction_errors:
        phase = files["helper/cursor_phase.go"]
        before = b"\t\tast, err := parser.NewParser(po).ParseExpr(text)\n\t\tif err != nil {\n\t\t\treturn err\n\t\t}"
        after = b"\t\tast, err := parser.NewParser(po).ParseExpr(text)\n\t\tif err != nil {\n\t\t\tout = append(out, constructionRecord(c.Request, state.Events, []map[string]any{}, err))\n\t\t\teng.Close()\n\t\t\tcontinue\n\t\t}"
        if phase.count(before) != 1:
            raise ValueError("cursor construction-error patch site changed")
        phase = phase.replace(before, after)
        before = b'\t\tif err != nil {\n\t\t\titem["construction_error"] = encodeError("construction", err)\n\t\t\titem["events"] = state.Events\n\t\t\tout = append(out, item)\n\t\t\teng.Close()\n\t\t\tcontinue\n\t\t}'
        after = b"\t\tif err != nil {\n\t\t\tout = append(out, constructionRecord(c.Request, state.Events, rawAST, err))\n\t\t\teng.Close()\n\t\t\tcontinue\n\t\t}"
        if phase.count(before) != 1:
            raise ValueError("cursor construction-error patch site changed")
        files["helper/cursor_phase.go"] = phase.replace(before, after)
        files["helper/cursor_errors.go"] = (
            HERE / "capture/cursor_errors.go"
        ).read_bytes()
    return files


def artifact_names(construction_errors=False):
    return set(helper_files(construction_errors)) | {
        "inputs.json",
        "source.json",
        "original.json",
        "original.stderr",
        "cursor-oracle",
        "toolchain.txt",
        "build-command.json",
        "build.stdout",
        "build.stderr",
    }


def wire_request(request):
    defaults = dict.fromkeys(
        ("time_ms", "start_ms", "end_ms", "step_ms", "lookback_ms", "subquery_step_ms"),
        0,
    )
    return defaults | request


def checked_original(document, inputs, *, result_validator=None):
    if (
        type(document) is not dict
        or document.keys() != PROFILE.keys() | {"cases"}
        or any(
            type(document.get(key)) is not type(value) or document.get(key) != value
            for key, value in PROFILE.items()
        )
        or type(document["cases"]) is not list
        or len(document["cases"]) != len(inputs)
    ):
        raise ValueError("original cursor profile or result roster changed")
    results = []
    for source, row in zip(inputs, document["cases"], strict=True):
        request = wire_request(source["request"])
        if (
            type(row) is not dict
            or row.keys()
            != {
                "request",
                "result",
                "events",
                "header",
                "raw_ast",
                "prepared_ast",
                "executed_ast",
            }
            or type(row["request"]) is not dict
            or row["request"] != request
            or any(
                type(row["request"][key]) is not type(value)
                for key, value in request.items()
            )
            or type(row["result"]) is not dict
            or row["result"].get("id") != source["request"]["id"]
            or any(
                type(row[key]) is not list
                for key in ("events", "raw_ast", "prepared_ast", "executed_ast")
            )
            or type(row["header"]) is not dict
        ):
            raise ValueError(
                "original cursor request, identity or phase evidence changed"
            )
        results.append(row["result"])
    expected = {"schema_version": 1, "results": results}
    try:
        (checked_results if result_validator is None else result_validator)(expected)
    except (KeyError, TypeError) as error:
        raise ValueError("original cursor result is incomplete or malformed") from error
    return expected


def load_reference(
    reference, inputs=None, *, construction_errors=False, result_validator=None
):
    reference = directory(reference)
    binding = json.loads((reference / "binding.json").read_bytes())
    inputs = cases() if inputs is None else inputs
    if (
        type(binding) is not dict
        or binding.keys() != {"commit", "cases", "source_manifest_sha256", "files"}
        or binding["commit"] != COMMIT
        or type(binding["cases"]) is not int
        or binding["cases"] != len(inputs)
        or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]
        or type(binding["files"]) is not dict
        or binding["files"].keys() != artifact_names(construction_errors)
    ):
        raise ValueError("original cursor binding or artifact roster changed")
    for name, digest in binding["files"].items():
        path = reference / name
        if path.is_symlink() or sha(path) != digest:
            raise ValueError("original cursor artifact changed")
    helpers = helper_files(construction_errors)
    paths = list((reference / "helper").rglob("*"))
    if (
        any(path.is_symlink() for path in paths)
        or (reference / "helper").is_symlink()
        or {path.relative_to(reference).as_posix() for path in paths if path.is_file()}
        != helpers.keys()
    ):
        raise ValueError("original cursor helper roster changed")
    if (
        any((reference / name).read_bytes() != raw for name, raw in helpers.items())
        or (reference / "toolchain.txt").read_text()
        != "go version go1.27.1 linux/amd64\n"
        or json.loads((reference / "build-command.json").read_bytes())
        != ["go", "build", "-p", "4", "-o", "../cursor-oracle", "."]
        or json.loads((reference / "inputs.json").read_bytes()) != inputs
        or manifest_digest(json.loads((reference / "source.json").read_bytes()))
        != binding["source_manifest_sha256"]
    ):
        raise ValueError(
            "original cursor helper, dataset, faults or source binding changed"
        )
    return inputs, checked_original(
        json.loads((reference / "original.json").read_bytes()),
        inputs,
        result_validator=result_validator,
    )


def capture(
    upstream,
    inputs=None,
    subject="cursor-call",
    *,
    construction_errors=False,
    result_validator=None,
    process_runner=None,
):
    upstream = directory(upstream)
    source = verify_facade(upstream, upstream)
    if manifest_digest(source) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original cursor source changed")
    inputs = cases() if inputs is None else inputs
    run = subprocess.run if process_runner is None else process_runner
    task = directory(
        tempfile.mkdtemp(prefix=subject + "-capture-", dir=directory(HERE / ".tmp"))
    )
    for name, raw in helper_files(construction_errors).items():
        path = task / name
        path.parent.mkdir(parents=True, exist_ok=True)
        directory(path.parent)
        path.write_bytes(raw)
    private = task / "tmp"
    private.mkdir(mode=0o700)
    env = os.environ.copy() | {
        "GOMODCACHE": str(directory(upstream.parents[2])),
        "GOCACHE": str(directory(HERE / ".tmp/go-build")),
        "GOPROXY": "off",
        "GOSUMDB": "off",
        "GOTOOLCHAIN": "local",
        "GOWORK": "off",
        "GOFLAGS": "-mod=readonly",
        "GOAMD64": "v1",
        "TMPDIR": str(directory(private)),
        "GOTMPDIR": str(private),
    }
    toolchain = subprocess.check_output(["go", "version"], env=env, text=True).strip()
    if toolchain != "go version go1.27.1 linux/amd64":
        raise ValueError("original cursor compiler profile changed")
    (task / "toolchain.txt").write_text(toolchain + "\n")
    command = ["go", "build", "-p", "4", "-o", "../cursor-oracle", "."]
    (task / "build-command.json").write_text(json.dumps(command) + "\n")
    with (
        (task / "build.stdout").open("xb") as out,
        (task / "build.stderr").open("xb") as err,
    ):
        process = run(
            command,
            cwd=directory(task / "helper"),
            env=env,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode:
        raise ValueError(f"original cursor build failed; evidence: {task}")
    for name, value in (("inputs.json", inputs), ("source.json", source)):
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (
        (task / "original.json").open("xb") as out,
        (task / "original.stderr").open("xb") as err,
    ):
        process = run(
            [str(task / "cursor-oracle"), str(task / "inputs.json")],
            env=env,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode or verify_facade(upstream, upstream) != source:
        raise ValueError(
            f"original cursor capture failed or source changed; evidence: {task}"
        )
    checked_original(
        json.loads((task / "original.json").read_bytes()),
        inputs,
        result_validator=result_validator,
    )
    (task / "binding.json").write_text(
        json.dumps(
            {
                "commit": COMMIT,
                "cases": len(inputs),
                "source_manifest_sha256": manifest_digest(source),
                "files": {
                    name: sha(task / name)
                    for name in sorted(artifact_names(construction_errors))
                },
            },
            indent=2,
        )
        + "\n"
    )
    load_reference(
        task,
        inputs,
        construction_errors=construction_errors,
        result_validator=result_validator,
    )
    print(json.dumps({"capture": str(task), "cases": len(inputs)}), flush=True)
    return task


def candidate_packets(inputs, page_size):
    groups = [
        {"id": row["request"]["id"], "requests": [row["request"]]} for row in inputs
    ]
    rows = []
    for source, group in zip(inputs, groups, strict=True):
        # Fault messages use the fixture's UTF-8 string transport.
        rows.append(
            {
                "op": "dataset",
                "dataset_id": group["id"],
                "page_size": page_size,
                "dataset": dataset_hex(source["dataset"]),
                "faults": deepcopy(source.get("faults", {})),
            }
        )
        rows.append(request_hex(source["request"], group["id"]))
    return groups, rows


def query_order(ordered, request):
    value = ordered(request) if callable(ordered) else ordered
    if type(value) is not bool:
        raise ValueError("cursor ordering contract must be boolean")
    return value or request["mode"] == "range"


def qualify(
    reference,
    binary,
    page_size=1024,
    inputs=None,
    subject="cursor-call",
    exact_numeric=False,
    ordered=True,
    *,
    construction_errors=False,
):
    reference = directory(reference)
    binary = Path(binary).resolve(strict=True)
    inputs, expected = load_reference(
        reference, inputs, construction_errors=construction_errors
    )
    if type(page_size) is not int or page_size < 1:
        raise ValueError("page size must be positive")
    task = directory(
        tempfile.mkdtemp(prefix=subject + "-compare-", dir=directory(HERE / ".tmp"))
    )
    groups, rows = candidate_packets(inputs, page_size)
    (task / "requests.jsonl").write_text(
        "".join(json.dumps(row) + "\n" for row in rows)
    )
    with (
        (task / "requests.jsonl").open("rb") as inputs_stream,
        (task / "candidate.jsonl").open("xb") as out,
        (task / "candidate.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            [str(binary)],
            stdin=inputs_stream,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode:
        raise ValueError(f"cursor candidate failed; evidence: {task}")
    responses = [
        json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()
    ]
    actual = checked_candidate_packets(responses, groups, "cursor")
    report = compare(
        expected,
        actual,
        [
            {
                "id": row["request"]["id"],
                "ordered": query_order(ordered, row["request"]),
                "exact_numeric": exact_numeric,
            }
            for row in inputs
        ],
    )
    report.update(
        cases=len(inputs),
        page_size=page_size,
        binary_sha256=sha(binary),
        reference_sha256=sha(reference / "original.json"),
        input_sha256=sha(task / "requests.jsonl"),
        output_sha256=sha(task / "candidate.jsonl"),
    )
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {key: report[key] for key in ("cases", "semantic_pass", "bitwise_pass")}
            | {
                "failures": len(report["semantic_failures"]),
                "paired_bits": len(report["bitwise_differences"]),
                "evidence": str(task),
            }
        ),
        flush=True,
    )
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    parser.add_argument("--page-size", type=int, default=1024)
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream)
    else:
        if not arguments.reference or not arguments.binary:
            parser.error("comparison requires --reference and --binary")
        report = qualify(arguments.reference, arguments.binary, arguments.page_size)
        raise SystemExit(not report["bitwise_pass"])
