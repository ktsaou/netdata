"""Execute every captured expression; unsupported paths remain open qualification."""
import argparse
import base64
import hashlib
import json
import re
import subprocess
import tempfile
from collections import Counter, defaultdict
from copy import deepcopy
from pathlib import Path

from build import directory
from build_frontend import build
from capture import HERE, source_commands, validate_evaluation, verify_facade
from compare import checked_results, compare
from upstream_inventory import REFERENCE, inventory

# The initial combined capture predates the separate public parser-profile hook.
# This is the exact parser_test.go digest at repository revision 2c846ef1fb.
INITIAL_PARSER_HOOK_SHA256 = "b576307090eedb78920303bd8024313fed8ee62029b02e7c75951563fbfb5be9"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def load(capture, upstream):
    capture, upstream = directory(capture), directory(upstream)
    inventory(upstream)
    manifest = json.loads((capture / "manifest.json").read_bytes())
    if (manifest.get("schema_version") != 1 or manifest.get("reference") != "prometheus/prometheus"
            or manifest.get("commit") != REFERENCE):
        raise ValueError("expression capture reference changed")
    hashes = verify_facade(upstream, capture / "reference")
    if hashes != manifest["source_sha256"]:
        raise ValueError("expression capture source binding changed")
    added = {"promql/promqltest/netdata_capture_test.go":
             (HERE / "capture/evaluation_test.go").read_bytes()}
    if "promql/parser/netdata_capture_test.go" in manifest["added_sha256"]:
        name = "promql/parser/netdata_capture_test.go"
        raw = (HERE / "capture/parser_test.go").read_bytes()
        if manifest["added_sha256"][name] == INITIAL_PARSER_HOOK_SHA256:
            raw = (capture / "reference" / name).read_bytes()
            if hashlib.sha256(raw).hexdigest() != INITIAL_PARSER_HOOK_SHA256:
                raise ValueError("initial parser capture hook changed")
        added[name] = raw
    for package, target in [("wire", "netdatawire"), ("codec", "netdatacodec")]:
        for path in sorted((HERE / "oracle" / package).glob("*.go")):
            raw = path.read_bytes().replace(b'"netdata-promql-conformance-oracle/wire"',
                                           b'"github.com/prometheus/prometheus/internal/netdatawire"')
            added[f"internal/{target}/{path.name}"] = raw
    if {name: hashlib.sha256(raw).hexdigest() for name, raw in added.items()} != manifest["added_sha256"]:
        raise ValueError("expression capture hook binding changed")
    if any((capture / "reference" / name).read_bytes() != raw for name, raw in added.items()):
        raise ValueError("expression capture hook changed")
    paths = list((capture / "reference").rglob("*"))
    if any(path.is_symlink() for path in paths) or {path.relative_to(capture / "reference").as_posix()
            for path in paths if path.is_file()} != set(hashes) | set(added):
        raise ValueError("expression capture source roster changed")
    for name in ["evaluation-metadata.json", "evaluation.jsonl"]:
        if manifest["outputs"][name] != {"path": name, "sha256": digest(capture / name)}:
            raise ValueError("expression capture output digest changed")
    metadata = json.loads((capture / "evaluation-metadata.json").read_bytes())
    for field, value in [("go_version", "go1.27.1"), ("go_arch", "amd64"),
                         ("go_os", "linux"), ("unicode_version", "17.0.0")]:
        if metadata.get(field) != value:
            raise ValueError("expression capture toolchain profile changed")
    counts = validate_evaluation(metadata, capture, source_commands(upstream))
    entries = [json.loads(line) for line in (capture / "evaluation.jsonl").open()]
    return capture, upstream, hashes, entries, counts


def dataset_hex(dataset):
    dataset = deepcopy(dataset)
    for series in dataset["series"]:
        for label in series["labels"]:
            for key in ["name", "value"]:
                label[key] = base64.b64decode(label[key], validate=True).hex()
    return dataset


def request_hex(request, dataset_id):
    result = deepcopy(request)
    source = (base64.b64decode(result.pop("query_bytes"), validate=True)
              if "query_bytes" in result else result.pop("query").encode())
    if "query" in result:
        raise ValueError("ambiguous expression bytes")
    result.update(expression=source.hex(), dataset_id=dataset_id)
    return result


def byte_base64(value):
    if type(value) is not str or re.fullmatch(r"(?:[0-9a-fA-F]{2})*", value) is None:
        raise ValueError("invalid candidate hex bytes")
    return base64.b64encode(bytes.fromhex(value)).decode()


def result_base64(result):
    result = deepcopy(result)
    for row in result["rows"]:
        for label in row["labels"]:
            for key in ["name", "value"]:
                label[key] = byte_base64(label[key])
        for point in row["points"]:
            if point["kind"] == "string":
                point["string"] = byte_base64(point["string"])
    for key in ["warnings", "infos"]:
        result[key] = [byte_base64(value) for value in result[key]]
    if "error" in result:
        result["error"]["message"] = byte_base64(result["error"]["message"])
        for error in result["error"].get("parse_errors", []):
            error["message"] = byte_base64(error["message"])
    return result


def checked_candidate_packets(responses, groups, subject="query"):
    expected = [("dataset_id", group["id"]) if request is None else ("id", request["id"])
                for group in groups for request in [None, *group["requests"]]]
    if type(responses) is not list or len(responses) != len(expected):
        raise ValueError(f"candidate {subject} packet count changed")
    results = []
    for row, (key, ident) in zip(responses, expected):
        if type(row) is not dict or row.get(key) != ident:
            raise ValueError(f"candidate {subject} packet order or identity changed")
        if key == "dataset_id":
            if row != {key: ident}:
                raise ValueError(f"candidate {subject} acknowledgement changed")
        else:
            if not {"id", "result"} <= row.keys() <= {"id", "result", "observations"}:
                raise ValueError(f"candidate {subject} result is incomplete or unsupported")
            result = result_base64(row["result"])
            if result.get("id") != ident:
                raise ValueError(f"candidate {subject} inner result identity changed")
            results.append(result)
    actual = {"schema_version": 1, "results": results}
    checked_results(actual)
    return actual


def qualify_outputs(entries, outputs):
    if any(type(entry.get("exact_numeric")) is not bool for entry in entries):
        raise ValueError("expression lacks independently classified numeric contract")
    expected = {entry["request"]["id"]: entry for entry in entries}
    if len(expected) != len(entries):
        raise ValueError("duplicate required expression identity")
    actual, opened = {}, {}
    for row in outputs:
        ident = row.get("id")
        if ident not in expected or ident in actual or ident in opened:
            raise ValueError("unknown or duplicate candidate expression identity")
        if "unsupported" in row:
            if set(row) != {"id", "unsupported", "observations"}:
                raise ValueError("malformed unsupported expression outcome")
            opened[ident] = byte_base64(row["unsupported"])
        else:
            if set(row) not in [{"id", "result"}, {"id", "result", "observations"}]:
                raise ValueError("malformed candidate expression outcome")
            result = result_base64(row["result"])
            if result["id"] != ident:
                raise ValueError("candidate result identity differs")
            checked_results({"schema_version": 1, "results": [result]})
            actual[ident] = result
    if set(actual) | set(opened) != set(expected):
        raise ValueError("incomplete candidate expression roster")
    completed = [entry for entry in entries if entry["request"]["id"] in actual]
    if completed:
        comparison = compare(
            {"schema_version": 1, "results": [entry["result"] for entry in completed]},
            {"schema_version": 1, "results": [actual[entry["request"]["id"]] for entry in completed]},
            [{"id": entry["request"]["id"], "ordered": entry["source_command"]["ordered"]
              or entry["request"]["mode"] == "range", "exact_numeric": entry["exact_numeric"]}
             for entry in completed])
    else:
        comparison = {"semantic_failures": {}, "bitwise_differences": [], "unexpected_results": [],
                      "semantic_pass": True, "bitwise_pass": True}
    failures = comparison["semantic_failures"]
    dispositions = [{"id": ident, "status": "open" if ident in opened else "fail" if ident in failures else "pass"}
                    for ident in expected]
    return {"required": len(expected), "completed": len(actual), "passed": len(actual) - len(failures),
            "failed": len(failures), "open": opened, "dispositions": dispositions,
            "semantic_failures": failures, "bitwise_differences": comparison["bitwise_differences"],
            "completed_semantic_pass": comparison["semantic_pass"],
            "semantic_pass": comparison["semantic_pass"] and not opened,
            "bitwise_pass": comparison["bitwise_pass"] and not opened}, actual


def numeric_contracts(entries, oracle, task):
    requests = task / "numeric-requests.json"
    requests.write_text(json.dumps([entry["request"] for entry in entries]) + "\n")
    with (task / "numeric-reference.json").open("wb") as output, (task / "numeric-reference.stderr").open("wb") as errors:
        process = subprocess.run([str(oracle), "--analyze", str(requests)], stdout=output, stderr=errors,
                                 timeout=600, check=False)
    if process.returncode:
        raise RuntimeError(f"reference numeric analysis failed ({process.returncode})")
    document = json.loads((task / "numeric-reference.json").read_bytes())
    for field, value in [("schema_version", 1), ("commit", REFERENCE), ("go_version", "go1.27.1"),
                         ("go_arch", "amd64"), ("go_os", "linux"), ("unicode_version", "17.0.0")]:
        if document.get(field) != value:
            raise ValueError("numeric analysis reference profile changed")
    rows = document["analyses"]
    if len(rows) != len(entries):
        raise ValueError("incomplete numeric analysis roster")
    for entry, row in zip(entries, rows, strict=True):
        if (row["id"] != entry["request"]["id"] or row.get("error") is not None
                or type(row.get("exact_numeric")) is not bool):
            raise ValueError("numeric analysis identity, parse status or contract changed")
        entry["exact_numeric"] = row["exact_numeric"]


def qualify(capture, upstream, oracle, binary=None, page_size=1024):
    if type(page_size) is not int or page_size < 1:
        raise ValueError("fixture page size must be positive")
    capture, upstream, hashes, entries, counts = load(capture, upstream)
    oracle = Path(oracle).expanduser().resolve(strict=True)
    binary = Path(binary).expanduser().resolve(strict=True) if binary else build().with_name("promql-query-fixture")
    parent = directory(HERE / ".tmp")
    task = directory(tempfile.mkdtemp(prefix="query-compare-", dir=parent))
    numeric_contracts(entries, oracle, task)
    grouped = defaultdict(list)
    for entry in entries:
        grouped[entry["dataset_id"]].append(entry)
    with (task / "requests.jsonl").open("w") as stream:
        for ident, group in grouped.items():
            dataset = json.loads((capture / "datasets" / (ident + ".json")).read_bytes())
            stream.write(json.dumps({"op": "dataset", "dataset_id": ident, "page_size": page_size,
                                     "dataset": dataset_hex(dataset)}, separators=(",", ":")) + "\n")
            for entry in group:
                stream.write(json.dumps(request_hex(entry["request"], ident), separators=(",", ":")) + "\n")
    with (task / "requests.jsonl").open("rb") as inputs, (task / "candidate.jsonl").open("wb") as output, \
            (task / "candidate.stderr").open("wb") as errors:
        process = subprocess.run([str(binary)], stdin=inputs, stdout=output, stderr=errors, timeout=600, check=False)
    if process.returncode:
        raise RuntimeError(f"expression candidate failed ({process.returncode}); evidence: {task.name}")
    responses = [json.loads(line) for line in (task / "candidate.jsonl").open()]
    acknowledgements = [row for row in responses if "dataset_id" in row]
    if acknowledgements != [{"dataset_id": ident} for ident in grouped]:
        raise ValueError("candidate dataset acknowledgement roster changed")
    report, actual = qualify_outputs(entries, [row for row in responses if "dataset_id" not in row])
    if verify_facade(upstream, capture / "reference") != hashes:
        raise ValueError("reference source changed during expression qualification")
    report.update(page_size=page_size, counts=counts, binary_sha256=digest(binary),
                  oracle_sha256=digest(oracle), exact_numeric_cases=sum(entry["exact_numeric"] for entry in entries),
                  numeric_request_sha256=digest(task / "numeric-requests.json"),
                  numeric_reference_sha256=digest(task / "numeric-reference.json"),
                  capture_manifest_sha256=digest(capture / "manifest.json"),
                  input_sha256=digest(task / "requests.jsonl"), candidate_sha256=digest(task / "candidate.jsonl"))
    (task / "candidate-wire.json").write_text(json.dumps({"schema_version": 1, "results": list(actual.values())}) + "\n")
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    reasons = Counter(base64.b64decode(reason).decode("utf-8", errors="backslashreplace") for reason in report["open"].values())
    print(json.dumps({key: report[key] for key in ["required", "completed", "passed", "failed", "semantic_pass", "bitwise_pass"]}
                     | {"open": len(report["open"]), "open_reasons": reasons,
                        "bitwise_differences": len(report["bitwise_differences"]),
                        "evidence": task.relative_to(HERE.parent.parent).as_posix()}), flush=True)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", required=True, help="complete verified builtin evaluation capture")
    parser.add_argument("--upstream", required=True, help="existing pinned Prometheus module source")
    parser.add_argument("--oracle", required=True, help="built pinned Go reference with numeric-contract analysis")
    parser.add_argument("--binary", help="existing Rust query fixture binary")
    parser.add_argument("--page-size", type=int, default=1024, help="test provider page size, independent of query limits")
    parser.add_argument("--strict-bits", action="store_true", help="also fail every paired candidate binary64 difference")
    args = parser.parse_args()
    report = qualify(args.capture, args.upstream, args.oracle, args.binary, args.page_size)
    raise SystemExit(not report["bitwise_pass" if args.strict_bits else "semantic_pass"])
