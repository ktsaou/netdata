"""Capture the pinned label sampler and its separately identified uint64 primitive."""

import argparse
import json
import math
import os
import re
import struct
import subprocess
import tempfile
from pathlib import Path

import cursor_calls
from build import directory
from capture import verify_facade
from count_values_cases import FIRST, SECOND
from histogram_reference import COMMIT, input_ledger
from info_cases import bits, series

HERE = Path(__file__).resolve().parent
COMMAND = ["go", "build", "-p", "4", "-o", "../sampler-oracle", "."]
HEX64 = re.compile(r"[0-9a-f]{16}\Z")
FIXED_RATIOS = (
    "0000000000000000",
    "8000000000000000",
    "3fe0000000000000",
    "bfe0000000000000",
    "3ff0000000000000",
    "bff0000000000000",
    "7ff0000000000000",
    "fff0000000000000",
    "7ff8000000000001",
)


def cases():
    labels = [
        {},
        {"__name__": "m", "z": "0"},
        {"__name__": "n", "z": "0"},
        {"z": "0"},
        {"__name__": "m", "z": "0", "empty": ""},
        {"a": FIRST},
        {"a": SECOND},
        {"__name__": "m", b"\xff\x00": b"\xff\x00"},
    ]
    labels += [
        {"__name__": "m", "data": "a" * size, "z": "0"}
        for size in (1, 254, 255, 256, 1024)
    ]
    result = [
        {"id": f"labels:{index}", "labels": series(row, [])["labels"], "hash": None}
        for index, row in enumerate(labels)
    ]
    integers = [
        0,
        1,
        (1 << 53) - 1,
        1 << 53,
        (1 << 53) + 1,
        (1 << 63) - 1,
        1 << 63,
        *[
            (1 << 64) - delta
            for delta in (2049, 2048, 1025, 1024, 1023, 513, 512, 511, 2, 1)
        ],
    ]
    result += [
        {"id": f"uint64:{value:016x}", "labels": None, "hash": f"{value:016x}"}
        for value in integers
    ]
    return result


def helper_files():
    files = cursor_calls.helper_files()
    files.pop("helper/cursor_phase.go")
    files["helper/ratio_sampler.go"] = (HERE / "capture/ratio_sampler.go").read_bytes()
    return files


def artifact_names():
    return set(helper_files()) | {
        "ratio_sampler.go",
        "inputs.json",
        "source.json",
        "original.json",
        "original.stderr",
        "sampler-oracle",
        "toolchain.txt",
        "build-command.json",
        "build.stdout",
        "build.stderr",
    }


def checked_original(document, inputs):
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
        raise ValueError("original sampler profile or roster changed")
    for request, row in zip(inputs, document["cases"], strict=True):
        if (
            type(row) is not dict
            or row.keys() != {"id", "hash", "offset", "thresholds"}
            or row["id"] != request["id"]
            or type(row["hash"]) is not str
            or HEX64.fullmatch(row["hash"]) is None
            or (request["hash"] is not None and row["hash"] != request["hash"])
            or type(row["thresholds"]) is not list
            or len(row["thresholds"]) != 15
        ):
            raise ValueError(
                "original sampler identity, hash or threshold roster changed"
            )
        # Prometheus engine.go:4835-4839 casts each unsigned operand before division.
        offset = float(int(row["hash"], 16)) / float((1 << 64) - 1)
        if row["offset"] != bits(offset):
            raise ValueError("original sampler unsigned normalization changed")
        ratios = list(FIXED_RATIOS)
        for boundary in (offset, offset - 1):
            ratios += [
                bits(math.nextafter(boundary, -math.inf)),
                bits(boundary),
                bits(math.nextafter(boundary, math.inf)),
            ]
        for threshold, expected_bits in zip(row["thresholds"], ratios, strict=True):
            if (
                type(threshold) is not dict
                or threshold.keys() != {"ratio", "selected"}
                or threshold["ratio"] != expected_bits
                or type(threshold["selected"]) is not bool
            ):
                raise ValueError(
                    "original sampler threshold bits or boolean type changed"
                )
            ratio = struct.unpack(">d", bytes.fromhex(expected_bits))[0]
            # These are the literal inequalities in Prometheus engine.go:4884-4885.
            selected = (ratio >= 0 and offset < ratio) or (
                ratio < 0 and offset >= 1 + ratio
            )
            if threshold["selected"] != selected:
                raise ValueError("original sampler threshold branch changed")
    return document["cases"]


def load_reference(reference):
    reference = directory(reference)
    binding = json.loads((reference / "binding.json").read_bytes())
    inputs = cases()
    if (
        type(binding) is not dict
        or binding.keys() != {"commit", "cases", "source_manifest_sha256", "files"}
        or binding["commit"] != COMMIT
        or type(binding["cases"]) is not int
        or binding["cases"] != len(inputs)
        or binding["source_manifest_sha256"] != input_ledger()["source_manifest_sha256"]
        or type(binding["files"]) is not dict
        or binding["files"].keys() != artifact_names()
    ):
        raise ValueError("original sampler binding or artifact roster changed")
    for name, digest in binding["files"].items():
        path = reference / name
        if (
            path.is_symlink()
            or type(digest) is not str
            or re.fullmatch(r"[0-9a-f]{64}", digest) is None
            or cursor_calls.sha(path) != digest
        ):
            raise ValueError("original sampler artifact changed")
    helpers = helper_files()
    paths = list((reference / "helper").rglob("*"))
    if (
        any(path.is_symlink() for path in paths)
        or (reference / "helper").is_symlink()
        or {path.relative_to(reference).as_posix() for path in paths if path.is_file()}
        != helpers.keys()
    ):
        raise ValueError("original sampler helper roster changed")
    if (
        any((reference / name).read_bytes() != raw for name, raw in helpers.items())
        or (reference / "ratio_sampler.go").read_bytes()
        != helpers["helper/ratio_sampler.go"]
        or (reference / "toolchain.txt").read_text()
        != "go version go1.27.1 linux/amd64\n"
        or json.loads((reference / "build-command.json").read_bytes()) != COMMAND
        or json.loads((reference / "inputs.json").read_bytes()) != inputs
        or cursor_calls.manifest_digest(
            json.loads((reference / "source.json").read_bytes())
        )
        != binding["source_manifest_sha256"]
    ):
        raise ValueError(
            "original sampler helper, input, compiler or source binding changed"
        )
    return inputs, checked_original(
        json.loads((reference / "original.json").read_bytes()), inputs
    )


def candidate_packets(original):
    return [
        {
            "id": f"{row['id']}:{index}",
            "mode": "limit-ratio-sampler",
            "borrowed": False,
            "request": {"hash": row["hash"], "ratio": threshold["ratio"]},
        }
        for row in original
        for index, threshold in enumerate(row["thresholds"])
    ]


def qualify_outputs(original, responses):
    expected = [
        {
            "id": f"{row['id']}:{index}",
            "actual": {"offset": row["offset"], "selected": threshold["selected"]},
        }
        for row in original
        for index, threshold in enumerate(row["thresholds"])
    ]
    if type(responses) is not list or len(responses) != len(expected):
        raise ValueError("sampler candidate packet roster changed")
    failures = []
    for source, actual in zip(expected, responses, strict=True):
        if (
            type(actual) is not dict
            or actual.keys() != {"id", "actual"}
            or actual["id"] != source["id"]
            or type(actual["actual"]) is not dict
            or actual["actual"].keys() != {"offset", "selected"}
            or type(actual["actual"]["offset"]) is not str
            or HEX64.fullmatch(actual["actual"]["offset"]) is None
            or type(actual["actual"]["selected"]) is not bool
        ):
            raise ValueError(
                "sampler candidate identity, offset or boolean packet changed"
            )
        if source != actual:
            failures.append(
                {
                    "id": source["id"],
                    "expected": source["actual"],
                    "actual": actual["actual"],
                }
            )
    return {
        "required": len(expected),
        "passed": len(expected) - len(failures),
        "failures": failures,
        "status": "FAIL" if failures else "PASS",
    }


def qualify(reference, binary):
    reference = directory(reference)
    _, original = load_reference(reference)
    binary = Path(binary).resolve(strict=True)
    digest = cursor_calls.sha(binary)
    task = directory(
        tempfile.mkdtemp(prefix="ratio-sampler-compare-", dir=directory(HERE / ".tmp"))
    )
    (task / "requests.jsonl").write_text(
        "".join(json.dumps(row) + "\n" for row in candidate_packets(original))
    )
    with (
        (task / "requests.jsonl").open("rb") as data,
        (task / "candidate.jsonl").open("xb") as out,
        (task / "candidate.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            [str(binary)], stdin=data, stdout=out, stderr=err, timeout=600, check=False
        )
    if process.returncode or cursor_calls.sha(binary) != digest:
        raise ValueError(
            f"sampler candidate failed or binary changed; evidence: {task}"
        )
    responses = [
        json.loads(line) for line in (task / "candidate.jsonl").read_text().splitlines()
    ]
    report = qualify_outputs(original, responses)
    report.update(
        binary_sha256=digest,
        reference_sha256=cursor_calls.sha(reference / "original.json"),
        input_sha256=cursor_calls.sha(task / "requests.jsonl"),
        output_sha256=cursor_calls.sha(task / "candidate.jsonl"),
    )
    (task / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {key: report[key] for key in ("required", "passed", "status")}
            | {"evidence": str(task)}
        ),
        flush=True,
    )
    return report


def capture(upstream):
    upstream = directory(upstream)
    source = verify_facade(upstream, upstream)
    if cursor_calls.manifest_digest(source) != input_ledger()["source_manifest_sha256"]:
        raise ValueError("original sampler source changed")
    task = directory(
        tempfile.mkdtemp(prefix="ratio-sampler-capture-", dir=directory(HERE / ".tmp"))
    )
    helpers = helper_files()
    for name, raw in helpers.items():
        path = task / name
        path.parent.mkdir(parents=True, exist_ok=True)
        directory(path.parent)
        path.write_bytes(raw)
    (task / "ratio_sampler.go").write_bytes(helpers["helper/ratio_sampler.go"])
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
    toolchain = subprocess.check_output(["go", "version"], env=env, text=True)
    if toolchain != "go version go1.27.1 linux/amd64\n":
        raise ValueError("original sampler compiler profile changed")
    (task / "toolchain.txt").write_text(toolchain)
    (task / "build-command.json").write_text(json.dumps(COMMAND) + "\n")
    with (
        (task / "build.stdout").open("xb") as out,
        (task / "build.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            COMMAND,
            cwd=directory(task / "helper"),
            env=env,
            stdout=out,
            stderr=err,
            timeout=600,
            check=False,
        )
    if process.returncode:
        raise ValueError(f"original sampler build failed; evidence: {task}")
    inputs = cases()
    for name, value in (("inputs.json", inputs), ("source.json", source)):
        (task / name).write_text(json.dumps(value, indent=2) + "\n")
    with (
        (task / "inputs.json").open("rb") as data,
        (task / "original.json").open("xb") as out,
        (task / "original.stderr").open("xb") as err,
    ):
        process = subprocess.run(
            [str(task / "sampler-oracle")],
            stdin=data,
            stdout=out,
            stderr=err,
            env=env,
            timeout=600,
            check=False,
        )
    if process.returncode or verify_facade(upstream, upstream) != source:
        raise ValueError(
            f"original sampler capture failed or source changed; evidence: {task}"
        )
    checked_original(json.loads((task / "original.json").read_bytes()), inputs)
    (task / "binding.json").write_text(
        json.dumps(
            {
                "commit": COMMIT,
                "cases": len(inputs),
                "source_manifest_sha256": cursor_calls.manifest_digest(source),
                "files": {
                    name: cursor_calls.sha(task / name)
                    for name in sorted(artifact_names())
                },
            },
            indent=2,
        )
        + "\n"
    )
    load_reference(task)
    print(
        json.dumps(
            {"capture": str(task), "cases": len(inputs), "thresholds": len(inputs) * 15}
        ),
        flush=True,
    )
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", action="store_true")
    parser.add_argument("--upstream")
    parser.add_argument("--reference")
    parser.add_argument("--binary")
    arguments = parser.parse_args()
    if arguments.capture:
        if not arguments.upstream:
            parser.error("capture requires --upstream")
        capture(arguments.upstream)
    else:
        if not arguments.reference:
            parser.error("validation requires --reference")
        if arguments.binary:
            raise SystemExit(
                qualify(arguments.reference, arguments.binary)["status"] != "PASS"
            )
        inputs, _ = load_reference(arguments.reference)
        print(
            json.dumps({"cases": len(inputs), "thresholds": len(inputs) * 15}),
            flush=True,
        )
