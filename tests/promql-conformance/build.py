#!/usr/bin/env python3
"""Build the pinned reference offline, using existing module sources."""
import argparse
import os
from pathlib import Path
import shlex
import subprocess

HERE = Path(__file__).resolve().parent


def directory(path):
    result = Path(path).expanduser().resolve(strict=True)
    if not result.is_dir() or len(result.parts) < 4:
        raise ValueError(f"unsafe directory: {path}")
    return result


def run(command, cwd, env):
    print(f"{cwd} > {shlex.join(map(str, command))}", flush=True)
    result = subprocess.run(list(map(str, command)), cwd=cwd, env=env)
    if result.returncode:
        raise SystemExit(f"command failed ({result.returncode}): {shlex.join(map(str, command))}")


def build(mod_cache, test=True):
    directory(HERE)
    output = HERE / ".tmp"
    output.mkdir(exist_ok=True)
    directory(output)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(directory(mod_cache)), GOCACHE=str(output / "go-build"),
               GOPROXY="off", GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly")
    oracle = HERE / "oracle"
    run(["go", "mod", "verify"], oracle, env)
    run(["go", "build", "-p", "4", "-o", output / "oracle", "."], oracle, env)
    if test:
        run(["go", "test", "-p", "4", "-count=1", "./..."], oracle, env)
    return output / "oracle"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mod-cache", required=True, help="existing Go module cache; downloads are disabled")
    parser.add_argument("--no-test", action="store_true")
    args = parser.parse_args()
    build(args.mod_cache, not args.no_test)
