#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Capture the unchanged pinned Prometheus limit helper using the existing Go toolchain."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
COMMIT = "5241a27fe3c6983549fccc32f6e65917408c63cd"
SOURCE_SHA256 = "5bd89a463770edd6f12bc3691e01e36a06f9362be67afa45ec2b1af3e9602756"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, type=Path, help="Existing pinned Prometheus web/api/v1/api.go")
    parser.add_argument("--go", required=True, type=Path, help="Existing Go1.27.1 executable")
    parser.add_argument("--output", type=Path, default=HERE / "api-limit-original.json")
    args = parser.parse_args()
    source = args.source.resolve(strict=True).read_bytes()
    if digest(source) != SOURCE_SHA256:
        raise ValueError("api.go differs from the pinned Prometheus source")
    begin = source.index(b"func parseLimitParam(")
    position = source.index(b"{", begin)
    depth = 1
    end = position + 1
    while depth:
        if source[end] == ord("{"):
            depth += 1
        elif source[end] == ord("}"):
            depth -= 1
        end += 1
    original = source[begin:end]
    parent = (HERE / ".tmp").resolve(strict=True)
    if not parent.is_dir() or not parent.is_relative_to(HERE.resolve()) or len(parent.parts) < 4:
        raise ValueError("invalid project capture directory")
    task = Path(tempfile.mkdtemp(prefix="api-limit-original-", dir=parent))
    inputs = [
        b"", b"0", b"-0", b"+0", b"1", b"+0007", b"-1", b"-0001", b"2147483648",
        b"9223372036854775807", b"9223372036854775808", b"-9223372036854775808", b"-9223372036854775809",
        b"18446744073709551615", b"18446744073709551616", b"9223372036854775808x", b"184467440737095516160x",
        b"0000000000000000000000000000000007", b"+", b"-", b"++1", b"--1", b" 1", b"1 ", b"1.0", b"1e3",
        b"0x10", b"1_0", b"\x00", b"\xff", b"a\n\"\\", "١".encode(), "日本".encode(), b"0" * 8192 + b"1",
    ]
    cases = [{"id": f"limit-{index:03d}", "input": base64.b64encode(value).decode()} for index, value in enumerate(inputs)]
    (task / "inputs.json").write_text(json.dumps(cases) + "\n")
    program = '''// SPDX-License-Identifier: Apache-2.0
package main
import ("encoding/base64"; "encoding/json"; "errors"; "fmt"; "os"; "runtime"; "strconv")
''' + original.decode() + '''
type Input struct { ID string `json:"id"`; Input string `json:"input"` }
func main() {
 if runtime.Version() != "go1.27.1" || runtime.GOARCH != "amd64" { panic("capture requires pinned Go1.27.1 AMD64") }
 var inputs []Input
 if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil { panic(err) }
 for _, input := range inputs {
  raw, err := base64.StdEncoding.DecodeString(input.Input); if err != nil { panic(err) }
  value, err := parseLimitParam(string(raw))
  row := map[string]any{"id": input.ID, "op": "limit", "input": input.Input, "success": err == nil}
  if err == nil { row["limit"] = strconv.FormatInt(int64(value), 10) } else { row["error_message"] = base64.StdEncoding.EncodeToString([]byte(err.Error())) }
  encoded, err := json.Marshal(row); if err != nil { panic(err) }; fmt.Println(string(encoded))
 }
}
'''
    (task / "main.go").write_text(program)
    command = [str(args.go.resolve(strict=True)), "run", str(task / "main.go")]
    print(shlex.join(command), flush=True)
    environment = os.environ.copy()
    environment.update(GOTOOLCHAIN="local", GOPROXY="off", GOSUMDB="off", GOWORK="off")
    with (task / "inputs.json").open("rb") as stdin, (task / "stdout").open("xb") as stdout, (task / "stderr").open("xb") as stderr:
        process = subprocess.Popen(command, cwd=task, env=environment, stdin=stdin, stdout=stdout, stderr=stderr)
        (task / "process.json").write_text(json.dumps({"pid": process.pid, "command": command}) + "\n")
        code = process.wait()
    if code:
        raise RuntimeError(f"original capture failed; see {task / 'stderr'}")
    rows = [json.loads(line) for line in (task / "stdout").read_text().splitlines()]
    if len(rows) != len(cases) or [row["id"] for row in rows] != [row["id"] for row in cases]:
        raise ValueError("original capture roster changed")
    result = {
        "schema_version": 1,
        "reference": {"repository": "prometheus/prometheus", "commit": COMMIT,
                      "module": "github.com/prometheus/prometheus@v0.315.0", "go": "go1.27.1", "architecture": "amd64",
                      "source": "web/api/v1/api.go", "source_sha256": digest(source), "helper_sha256": digest(original)},
        "helpers": rows, "selectors": [], "selector_labelsets": [],
    }
    args.output.write_text(json.dumps(result, indent=2, ensure_ascii=True) + "\n")
    print(json.dumps({"original_cases": len(rows), "output": str(args.output), "sha256": digest(args.output.read_bytes())}), flush=True)


if __name__ == "__main__":
    main()
