"""Qualify eight declared 9.A OR boundaries without losing derived Matrix flags."""

import argparse
import json
import os
import shutil
import subprocess
import tempfile
from copy import deepcopy
from pathlib import Path

import cursor_calls
import quantile_aggregate_calls
from build import directory
from capture import verify_facade
from compare import checked_results, compare, labels
from histogram_reference import COMMIT, input_ledger
from query import checked_candidate_packets

HERE = Path(__file__).resolve().parent
HOOK = "promql/netdata_quantile_policy_fixture.go"
BINDINGS = {
    "cursor-oracle",
    "binding.json",
    "source.json",
    "original.json",
    "inputs.json",
}


def declarations(inputs):
    by_id = {row["request"]["id"]: row for row in inputs}
    selected = []
    for first, second in (("float", "float_histogram"), ("float_histogram", "float")):
        for order, body in enumerate(
            ('m{z="a"} or (m{z="b"}*1)', '(m{z="b"}*1) or m{z="a"}')
        ):
            for query_index, query in (
                (0, f"quantile by(__name__)(0.5,{body})"),
                (2, f"quantile by(__name__)(0.5,({body}) or n)"),
            ):
                ident = (
                    f"aggregate-quantile:native-flags-{first}-{second}-{order}:"
                    f"upstream-tests:{query_index}:3"
                )
                row = by_id[ident]
                if row.get("faults") or row["request"]["query"] != query:
                    raise ValueError("declared QUANTILE map boundary changed")
                selected.append(deepcopy(row))
    return selected


def helper_files():
    files = {
        f"helper/{path.relative_to(HERE / 'oracle').as_posix()}": path.read_bytes()
        for path in sorted((HERE / "oracle").rglob("*.go"))
        if not path.name.endswith("_test.go")
    }
    main = files["helper/main.go"]
    if main.count(b"func main() {") != 1:
        raise ValueError("canonical oracle entry point changed")
    files["helper/main.go"] = main.replace(b"func main() {", b"func oracleMain() {")
    files["helper/go.sum"] = (HERE / "oracle/go.sum").read_bytes()
    files["helper/go.mod"] = (HERE / "oracle/go.mod").read_bytes() + (
        b"\nreplace github.com/prometheus/prometheus => ../prometheus-reference\n"
    )
    files["helper/quantile_policy_main.go"] = (
        HERE / "capture/quantile_policy_main.go"
    ).read_bytes()
    files[f"prometheus-reference/{HOOK}"] = (
        HERE / "capture/quantile_policy.go.in"
    ).read_bytes()
    return files


def artifact_names():
    return set(helper_files()) | {
        "inputs.json",
        "source.json",
        "original.json",
        "original.stderr",
        "quantile-policy-oracle",
        "toolchain.txt",
        "build-command.json",
        "build.stdout",
        "build.stderr",
    }


def checked_source(document, inputs):
    if (
        type(document) is not dict
        or document.keys() != cursor_calls.PROFILE.keys() | {"cases"}
        or any(
            type(document.get(key)) is not type(value) or document.get(key) != value
            for key, value in cursor_calls.PROFILE.items()
        )
        or type(document["cases"]) is not list
        or len(document["cases"]) != len(inputs)
    ):
        raise ValueError("QUANTILE policy profile or result roster changed")
    expected = {"schema_version": 1, "results": []}
    for source, row in zip(inputs, document["cases"], strict=True):
        if (
            type(row) is not dict
            or row.keys() != {"request", "result", "child", "child_drop_names"}
            or row["request"] != cursor_calls.wire_request(source["request"])
            or type(row["request"]) is not dict
            or any(
                type(row["request"][key]) is not type(value)
                for key, value in cursor_calls.wire_request(source["request"]).items()
            )
            or type(row["result"]) is not dict
            or row["result"].get("id") != source["request"]["id"]
            or type(row["child"]) is not dict
            or row["child"].get("id") != "child:" + source["request"]["id"]
            or type(row["child_drop_names"]) is not list
            or any(type(flag) is not bool for flag in row["child_drop_names"])
        ):
            raise ValueError("QUANTILE policy scope, identity or flags changed")
        child = row["child"]
        checked_results({"schema_version": 1, "results": [child, row["result"]]})
        identities = [labels(series) for series in child["rows"]]
        if (
            child["kind"] != "matrix"
            or child["warnings"]
            or child["infos"]
            or len(identities) != len(row["child_drop_names"])
            or identities != sorted(set(identities))
            or row["child_drop_names"]
            != [dict(identity).get(b"z") == b"b" for identity in identities]
        ):
            raise ValueError("QUANTILE policy child is not a complete canonical Matrix")
        expected["results"].append(row["result"])
    checked_results(expected)
    return expected


def load(reference, base):
    reference, base = directory(reference), directory(base)
    inputs, _ = quantile_aggregate_calls.load_reference(base)
    selected = declarations(inputs)
    manifest = json.loads((reference / "manifest.json").read_bytes())
    if (
        type(manifest) is not dict
        or manifest.keys()
        != {"commit", "required", "base_ids", "original_bindings", "files"}
        or manifest["commit"] != COMMIT
        or type(manifest["required"]) is not int
        or manifest["required"] != len(selected)
        or manifest["base_ids"] != [row["request"]["id"] for row in selected]
        or type(manifest["original_bindings"]) is not dict
        or manifest["original_bindings"].keys() != BINDINGS
        or type(manifest["files"]) is not dict
        or manifest["files"].keys() != artifact_names()
    ):
        raise ValueError("QUANTILE policy source binding or boundary roster changed")
    for name, digest in manifest["original_bindings"].items():
        if cursor_calls.sha(base / name) != digest:
            raise ValueError("QUANTILE policy original source changed")
    for name, digest in manifest["files"].items():
        path = reference / name
        if path.is_symlink() or cursor_calls.sha(path) != digest:
            raise ValueError("QUANTILE policy source artifact changed")
    if json.loads((reference / "inputs.json").read_bytes()) != selected:
        raise ValueError("QUANTILE policy complete input changed")
    if (reference / "toolchain.txt").read_text() != "go version go1.27.1 linux/amd64\n":
        raise ValueError("QUANTILE policy compiler profile changed")
    if json.loads((reference / "build-command.json").read_bytes()) != [
        "go",
        "build",
        "-p",
        "4",
        "-o",
        "../quantile-policy-oracle",
        ".",
    ]:
        raise ValueError("QUANTILE policy build changed")
    facade = directory(reference / "prometheus-reference")
    original_source = json.loads((base / "source.json").read_bytes())
    if json.loads((reference / "source.json").read_bytes()) != original_source:
        raise ValueError("QUANTILE policy production source roster changed")
    paths = list(facade.rglob("*"))
    if (
        any(path.is_symlink() for path in paths)
        or {path.relative_to(facade).as_posix() for path in paths if path.is_file()}
        != original_source.keys() | {HOOK}
        or any(
            cursor_calls.sha(facade / name) != digest
            for name, digest in original_source.items()
        )
    ):
        raise ValueError("QUANTILE policy modified an original source owner")
    helpers = helper_files()
    paths = list((reference / "helper").rglob("*"))
    if (
        (reference / "helper").is_symlink()
        or any(path.is_symlink() for path in paths)
        or {path.relative_to(reference).as_posix() for path in paths if path.is_file()}
        != {name for name in helpers if name.startswith("helper/")}
        or any((reference / name).read_bytes() != raw for name, raw in helpers.items())
    ):
        raise ValueError("QUANTILE policy helper changed")
    expected = checked_source(
        json.loads((reference / "original.json").read_bytes()), selected
    )
    return selected, expected


def capture(base, upstream):
    base, upstream = directory(base), directory(upstream)
    inputs, _ = quantile_aggregate_calls.load_reference(base)
    selected = declarations(inputs)
    source = verify_facade(upstream, upstream)
    if cursor_calls.manifest_digest(source) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("QUANTILE policy original source pin changed")
    task = directory(
        tempfile.mkdtemp(prefix="quantile-policy-source-", dir=directory(HERE / ".tmp"))
    )
    facade = task / "prometheus-reference"
    shutil.copytree(upstream, facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    verify_facade(upstream, facade)
    helpers = helper_files()
    for name, raw in helpers.items():
        path = task / name
        if path.exists():
            raise ValueError("QUANTILE policy helper would replace original source")
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
    version = subprocess.check_output(["go", "version"], env=env, text=True).strip()
    if version != "go version go1.27.1 linux/amd64":
        raise ValueError("QUANTILE policy compiler profile changed")
    (task / "toolchain.txt").write_text(version + "\n")
    command = ["go", "build", "-p", "4", "-o", "../quantile-policy-oracle", "."]
    (task / "build-command.json").write_text(json.dumps(command) + "\n")
    print(command, flush=True)
    with (
        (task / "build.stdout").open("xb") as out,
        (task / "build.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            command,
            cwd=directory(task / "helper"),
            env=env,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode:
        raise ValueError(f"QUANTILE policy original build failed; evidence: {task}")
    for name, value in (("inputs.json", selected), ("source.json", source)):
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (
        (task / "original.json").open("xb") as out,
        (task / "original.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            [str(task / "quantile-policy-oracle"), str(task / "inputs.json")],
            env=env,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode or verify_facade(upstream, facade) != source:
        raise ValueError(
            f"QUANTILE policy capture failed or production source changed; evidence: {task}"
        )
    checked_source(json.loads((task / "original.json").read_bytes()), selected)
    manifest = {
        "commit": COMMIT,
        "required": len(selected),
        "base_ids": [row["request"]["id"] for row in selected],
        "original_bindings": {name: cursor_calls.sha(base / name) for name in BINDINGS},
        "files": {
            name: cursor_calls.sha(task / name) for name in sorted(artifact_names())
        },
    }
    (task / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    load(task, base)
    print(
        json.dumps({"policy_capture": str(task), "boundaries": len(selected)}),
        flush=True,
    )
    return task


def expected_policy(raw, selected, controls):
    replacements = {row["id"]: row for row in controls["results"]}
    identities = {row["request"]["id"] for row in selected}
    checked_results(controls)
    if replacements.keys() != identities or len(replacements) != len(
        controls["results"]
    ):
        raise ValueError("QUANTILE policy control roster changed")
    expected = deepcopy(raw)
    found = set()
    for index, row in enumerate(expected["results"]):
        if row["id"] in replacements:
            expected["results"][index] = deepcopy(replacements[row["id"]])
            found.add(row["id"])
    if found != identities:
        raise ValueError("QUANTILE policy original roster is incomplete")
    return expected, sorted(found)


def qualify(base, reference, candidate):
    base, candidate = directory(base), directory(candidate)
    inputs, raw = quantile_aggregate_calls.load_reference(base)
    selected, controls = load(reference, base)
    expected, boundaries = expected_policy(raw, selected, controls)
    report = json.loads((candidate / "report.json").read_bytes())
    if (
        report["cases"] != len(inputs)
        or report["reference_sha256"] != cursor_calls.sha(base / "original.json")
        or report["output_sha256"] != cursor_calls.sha(candidate / "candidate.jsonl")
        or report["input_sha256"] != cursor_calls.sha(candidate / "requests.jsonl")
    ):
        raise ValueError("QUANTILE policy candidate packet changed")
    groups, requests = cursor_calls.candidate_packets(inputs, report["page_size"])
    if [
        json.loads(line)
        for line in (candidate / "requests.jsonl").read_text().splitlines()
    ] != requests:
        raise ValueError("QUANTILE policy candidate scope, profile or dataset changed")
    actual = checked_candidate_packets(
        [
            json.loads(line)
            for line in (candidate / "candidate.jsonl").read_text().splitlines()
        ],
        groups,
        "cursor",
    )
    cases = [
        {"id": row["request"]["id"], "ordered": True, "exact_numeric": True}
        for row in inputs
    ]
    original_report = compare(raw, actual, cases)
    for field in (
        "semantic_pass",
        "bitwise_pass",
        "semantic_failures",
        "bitwise_differences",
    ):
        if report[field] != original_report[field]:
            raise ValueError("QUANTILE policy raw report changed")
    qualified = compare(expected, actual, cases)
    qualified.update(
        cases=len(inputs),
        policy_boundaries=boundaries,
        raw_failures=original_report["semantic_failures"],
        source_manifest_sha256=cursor_calls.sha(Path(reference) / "manifest.json"),
        candidate_output_sha256=report["output_sha256"],
    )
    (candidate / "policy-report.json").write_text(
        json.dumps(qualified, indent=2) + "\n"
    )
    return qualified


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--base", required=True)
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--candidate")
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.base, arguments.upstream)
    else:
        if not arguments.reference or not arguments.candidate:
            parser.error("qualification requires reference and candidate directories")
        raise SystemExit(
            not qualify(arguments.base, arguments.reference, arguments.candidate)[
                "bitwise_pass"
            ]
        )
