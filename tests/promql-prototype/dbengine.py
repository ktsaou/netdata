#!/usr/bin/env python3
"""Build and run the isolated native retained-cursor fixture. Never installs a daemon."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

from build import HERE, REPO, run

ANCHOR_MS = 1700000000000

def build_native():
    base = HERE / "build"
    base.mkdir(parents=True, exist_ok=True)
    submodule = REPO / "src/aclk/aclk-schemas"
    pin = subprocess.check_output(["git", "ls-tree", "HEAD", "src/aclk/aclk-schemas"], cwd=REPO, text=True).split()[2]
    if not (submodule / ".git").exists():
        if any(submodule.iterdir()):
            raise RuntimeError("ACLK submodule directory is nonempty without Git metadata")
        bare = base / "aclk.git"
        if not bare.exists():
            url = subprocess.check_output(["git", "config", "-f", ".gitmodules", "submodule.aclk/aclk-schemas.url"], cwd=REPO, text=True).strip()
            run(["git", "clone", "--bare", url, bare])
        run(["git", f"--git-dir={bare}", "worktree", "add", "--detach", submodule, pin])
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=submodule, text=True).strip()
    if actual != pin:
        raise RuntimeError("ACLK worktree does not match the Netdata gitlink; refusing to alter it")
    options = ["DEFAULT_FEATURE_STATE=OFF", "ENABLE_DBENGINE=ON", "ENABLE_DASHBOARD=OFF", "ENABLE_ML=OFF",
               "ENABLE_PLUGIN_SCRIPTS=OFF", "ENABLE_PLUGIN_GO=OFF", "ENABLE_PLUGIN_OTEL=OFF", "ENABLE_PLUGIN_NETFLOW=OFF",
               "ENABLE_LIBBACKTRACE=OFF", "ENABLE_BUNDLED_PROTOBUF=OFF", "ENABLE_BUNDLED_JSONC=OFF", "ENABLE_BUNDLED_YAML=OFF",
               "CMAKE_BUILD_TYPE=Debug"]
    run(["cmake", "-S", REPO, "-B", base / "netdata", "-G", "Ninja", *[f"-D{o}" for o in options]])
    run(["cmake", "--build", base / "netdata", "--target", "netdata", "-j", "4"])

def export_fixture():
    parent = HERE / "build"
    parent.mkdir(parents=True, exist_ok=True)
    # Private directories remain for inspection; this runner never deletes them.
    root = Path(tempfile.mkdtemp(prefix="fixture-", dir=parent)).resolve(strict=True)
    if root.parent != parent.resolve(strict=True) or root.stat().st_mode & 0o077:
        raise RuntimeError("unsafe fixture root")
    names = ["cache", "lib", "log", "config", "stock", "web", "plugins"]
    for name in names:
        (root / name).mkdir(mode=0o700)
    (root / ".promql-prototype-fixture").write_text("isolated-promql-prototype\n")
    # Absolute paths are required by native safe-directory checks. The process alias
    # keeps machine-specific checkout paths out of the private configuration.
    config = "[directories]\n" + "".join(f"    {key} = /proc/self/cwd/{name}\n" for key, name in [
        ("cache", "cache"), ("lib", "lib"), ("log", "log"), ("config", "config"), ("stock config", "stock"),
        ("stock data", "stock"), ("web", "web"), ("plugins", "plugins")])
    config += "[db]\n    mode = ram\n    storage tiers = 1\n    dbengine page cache size = 16MiB\n    dbengine extent cache size = 16MiB\n"
    config += "[health]\n    enabled = no\n[registry]\n    enabled = no\n[cloud]\n    enabled = no\n[global]\n    libuv worker threads = 16\n"
    private = root / "config/netdata.conf"
    private.write_text(config)
    environment = os.environ.copy()
    environment["PROMQL_PROTOTYPE_FIXTURE_ROOT"] = str(root)
    command = [HERE / "build/netdata/netdata", "-c", private, "-W", "promql-dbengine-export"]
    from shlex import join
    print(join(map(str, command)), flush=True)
    result = subprocess.run(list(map(str, command)), cwd=root, env=environment, capture_output=True, text=True, timeout=120)
    (root / "stderr.log").write_text(result.stderr)
    (root / "snapshot.json").write_text(result.stdout)
    if result.returncode:
        raise RuntimeError(f"fixture exporter exit {result.returncode}; inspect {root / 'stderr.log'}\n{result.stderr[-4000:]}")
    dataset = json.loads(result.stdout)
    assert len(dataset["series"]) == 26, "each physical CPU row must expose two logical names"
    assert all(row["points"] for row in dataset["series"]), "empty retained metric"
    cases = json.loads((HERE / "cases.json").read_text())
    for case in cases:
        for key in ["time_ms", "start_ms", "end_ms"]:
            if key in case:
                case[key] += ANCHOR_MS
    fraction = next(row for row in dataset["series"] if row["labels"]["__name__"] == "retained_fraction")
    decoded = fraction["points"][-1][1]
    assert decoded != 1.23456789 and abs(decoded - 1.23456789) < 1e-5, "must observe the retained Tier 0 quantization"
    cases.append({"id": "D001", "query": "retained_fraction", "time_ms": ANCHOR_MS+600000})
    (root / "cases.json").write_text(json.dumps(cases, indent=2) + "\n")
    for row in dataset["series"]:
        assert all(ANCHOR_MS <= p[0] <= ANCHOR_MS + 600000 for p in row["points"])
    manifest = {"anchor_ms": ANCHOR_MS, "logical_series": len(dataset["series"]), "tiers": [0],
                "source": "storage engine acquired UUID handle and init/next/finalize cursor",
                "decode": "end_time_s * 1000, sum / count; gaps omitted; no interpolation",
                "quantization_probe": {"collected": 1.23456789, "decoded": decoded},
                "scope": "isolated Tier 0 readback, not restart, tier seams, Agent API or Cloud transport"}
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (parent / "latest-fixture.txt").write_text(root.name + "\n")
    run([os.sys.executable, HERE / "run.py", "--baseline-only", "--dataset", root / "snapshot.json", "--cases", root / "cases.json"])
    return root

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--build", action="store_true")
    args = parser.parse_args()
    if args.build:
        build_native()
    export_fixture()
