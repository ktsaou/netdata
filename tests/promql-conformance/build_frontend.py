#!/usr/bin/env python3
"""Build the standalone Rust frontend without changing the Agent/plugin build."""
import os
from pathlib import Path

from build import directory, run

HERE = Path(__file__).resolve().parent


def build():
    root = directory(HERE.parent.parent)
    crate = root
    output = HERE / ".tmp"
    output.mkdir(exist_ok=True)
    directory(output)
    env = os.environ.copy()
    env.update(CARGO_NET_OFFLINE="true", CARGO_TARGET_DIR=str(output / "core-target"))
    command = ["cargo", "build", "--offline", "--locked", "--features", "fixtures", "-j", "4", "--release"]
    run(command, crate, env)
    run(["cargo", "test", "--offline", "--locked", "--features", "fixtures", "-j", "4", "--release"], crate, env)
    return output / "core-target/release/promql-frontend-fixture"


if __name__ == "__main__":
    build()
