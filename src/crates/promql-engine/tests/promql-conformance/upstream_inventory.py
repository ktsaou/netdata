#!/usr/bin/env python3
"""Account for pinned upstream evaluation sources without treating them as passed tests."""
import argparse
import hashlib
import json
from pathlib import Path
import re

REFERENCE = "5241a27fe3c6983549fccc32f6e65917408c63cd"
HERE = Path(__file__).resolve().parent


def inventory(root):
    root = Path(root).expanduser().resolve(strict=True)
    if not root.is_dir() or len(root.parts) < 4:
        raise ValueError("unsafe upstream source directory")
    sources = []
    for path in sorted((root / "promql/promqltest/testdata").glob("*.test")):
        raw = path.read_bytes()
        text = raw.decode()
        sources.append({"path": path.relative_to(root).as_posix(), "sha256": hashlib.sha256(raw).hexdigest(),
            "physical_lines": len(text.splitlines()),
            "eval_commands": len(re.findall(r"^[ \t]*eval(?:_[a-z]+)?(?:\s|$)", text, re.M)),
            "ordinary_eval_commands": len(re.findall(r"^[ \t]*eval(?:\s|$)", text, re.M)),
            "negative_eval_commands": len(re.findall(r"^[ \t]*eval_fail(?:\s|$)", text, re.M)),
            "profile": "upstream-tests"})
    if len(sources) != 21 or sum(s["eval_commands"] for s in sources) != 2243:
        raise ValueError("upstream corpus differs from the reviewed v3.15 snapshot")
    parser_sources = []
    for name in ["parse_test.go", "lex_test.go"]:
        path = root / "promql/parser" / name
        parser_sources.append({"path": path.relative_to(root).as_posix(), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "import_requirement": "Include computed inputs and structured expectations; tables are unexported."})
    result = {"schema_version": 1, "reference": "prometheus/prometheus", "commit": REFERENCE,
        "evaluation_sources": sources, "parser_sources": parser_sources,
        "summary": {"files": len(sources), "eval_commands": sum(s["eval_commands"] for s in sources),
                    "ordinary_eval_commands": sum(s["ordinary_eval_commands"] for s in sources),
                    "negative_eval_commands": sum(s["negative_eval_commands"] for s in sources),
                    "physical_lines": sum(s["physical_lines"] for s in sources)},
        "qualification": "Inventory only. This does not execute or qualify the Rust engine."}
    expected = json.loads((HERE / "upstream-sources.json").read_text())
    if result != expected:
        raise ValueError("upstream source digests differ from the pinned manifest")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, help="existing pinned Prometheus module source directory")
    args = parser.parse_args()
    print(json.dumps(inventory(args.upstream), indent=2))
