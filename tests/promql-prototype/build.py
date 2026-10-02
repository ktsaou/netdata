#!/usr/bin/env python3
"""Build standalone C callers and independent native libraries using existing tools."""
import argparse
import os
from pathlib import Path
import shlex
import subprocess
import time

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]

def run(command, cwd=REPO, env=None):
    print(f"{cwd} > {shlex.join(map(str, command))}", flush=True)
    started = time.perf_counter()
    result = subprocess.run(list(map(str, command)), cwd=cwd, env=env)
    if result.returncode:
        raise SystemExit(f"command failed with exit status {result.returncode}: {shlex.join(map(str, command))}")
    return time.perf_counter() - started

def flags(*args):
    return shlex.split(subprocess.check_output(["pkg-config", *args], text=True))

def build(languages, sanitize=False):
    import json
    out = HERE / ("build/sanitize" if sanitize else "build/release")
    out.mkdir(parents=True, exist_ok=True)
    options = ["-g", "-Wall", "-Wextra", "-Werror", "-Wno-misleading-indentation", "-I", str(HERE)]
    options += ["-O1", "-fsanitize=address,undefined", "-fno-omit-frame-pointer"] if sanitize else ["-O2"]
    for name in ["host", "regex", "ownership"]:
        run(["gcc", "-std=c11", *options, *flags("--cflags", "json-c", "libpcre2-8"), "-c", HERE / f"{name}.c", "-o", out / f"{name}.o"])
    measurements = {}
    for language in languages:
        checkout = (REPO.parent / f"netdata-{language}").resolve(strict=True)
        if not (checkout / ".git").is_file():
            raise SystemExit(f"candidate must be a Git worktree: {checkout}")
        source = checkout / "tests/promql-prototype"
        lib = out / f"lib{language}.a"
        began = time.perf_counter()
        compiler_environment = os.environ.copy()
        compiler_environment["CCACHE_DISABLE"] = "1"
        if language == "rust":
            run(["rustc", "--edition", "2021", "--crate-name", "promql_rust", "--crate-type", "staticlib", "-C", "opt-level=1" if sanitize else "opt-level=2", "-C", "debuginfo=1", "-C", "panic=unwind", "-C", "debug-assertions=yes", source / "engine.rs", "-o", lib])
        else:
            compiler, standard, suffix = ("gcc", "c11", "c") if language == "c" else ("g++", "c++17", "cpp")
            run([compiler, f"-std={standard}", *options, "-c", source / f"engine.{suffix}", "-o", out / f"{language}.o"], env=compiler_environment)
            run(["ar", "rcs", lib, out / f"{language}.o"])
        measurements[language] = {"compile_seconds": time.perf_counter() - began, "archive_bytes": lib.stat().st_size}
        links = ["-lstdc++"] if language == "cpp" else []
        for name in ["host", "ownership"]:
            run(["gcc", *options, out / f"{name}.o", out / "regex.o", lib, *links, *flags("--libs", "json-c", "libpcre2-8"), "-lm", "-ldl", "-pthread", "-o", out / f"{language}-{name}"])
        measurements[language]["host_bytes"] = (out / f"{language}-host").stat().st_size
        run(["objcopy", "--strip-debug", out / f"{language}-host", out / f"{language}-host-stripped"])
        measurements[language]["host_stripped_debug_bytes"] = (out / f"{language}-host-stripped").stat().st_size
    (out / "build-metrics.json").write_text(json.dumps(measurements, indent=2) + "\n")
    return out

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("languages", nargs="*", choices=["c", "cpp", "rust"])
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--oracle", action="store_true")
    args = parser.parse_args()
    build(args.languages or ["c", "cpp", "rust"], args.sanitize)
    if args.oracle:
        oracle = HERE / "oracle"
        env = os.environ.copy()
        env.update(GOMODCACHE=str(oracle / ".cache/go-mod"), GOCACHE=str(oracle / ".cache/go-build"))
        run(["go", "build", "-p", "4", "-o", ".cache/oracle", "."], oracle, env)
        run(["go", "test", "-p", "4", "."], oracle, env)
