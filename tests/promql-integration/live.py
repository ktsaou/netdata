#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Exercise real HTTP queries over collected DBengine data in an isolated Debug Agent."""

import argparse
import gzip
import http.client
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
from urllib.parse import urlencode

from storage import REPO, make_fixture, safe_parent, sha256, source_binding

HOST_CASES = {"context-prepare", "context-step", "chart-prepare", "chart-step", "dimension-prepare",
              "dimension-step", "context-owner", "query-references", "unloaded-cancel", "loading-cancel",
              "loading-complete", "unloaded-reset"}


def configuration(root, codec):
    names = [("config", "config"), ("stock config", "stock"), ("stock data", "stock"),
             ("cache", "cache"), ("lib", "lib"), ("home", "lib"), ("log", "log"),
             ("web", "web"), ("plugins", "plugins")]
    text = "[directories]\n" + "".join(f"    {key} = {safe_parent(root / value)}\n" for key, value in names)
    text += "[global]\n    hostname = promql-fixture-main\n    libuv worker threads = 16\n"
    text += f"[db]\n    db = dbengine\n    update every = 1\n    storage tiers = 1\n    dbengine page type = {codec}\n"
    text += "    dbengine page cache size = 16MiB\n    dbengine extent cache size = 16MiB\n    dbengine tier 0 retention time = 0\n"
    text += "[web]\n    mode = static-threaded\n    bind to = 127.0.0.1:0\n    web server threads = 2\n"
    for feature in ["connections", "dashboard", "badges", "netdata.conf", "management"]:
        text += f"    allow {feature} from = localhost 127.0.0.1\n    allow {feature} by dns = no\n"
    text += "[logs]\n    daemon = stderr\n    collector = stderr\n    access = stderr\n    health = stderr\n    debug = stderr\n"
    text += "[health]\n    enabled = no\n[ml]\n    enabled = no\n[registry]\n    enabled = no\n"
    text += "[webrtc]\n    enabled = no\n[stream]\n    enabled = no\n[plugins]\n    enable running new plugins = no\n"
    for plugin in ["proc", "diskspace", "cgroups", "tc", "idlejitter", "statsd", "apps", "go.d", "charts.d",
                   "python.d", "debugfs", "perf", "slabinfo", "ioping", "ebpf", "systemd-journal", "network-viewer",
                   "timex", "profile"]:
        text += f"    {plugin} = no\n"
    (root / "config/netdata.conf").write_text(text)
    (root / "config/.opt-out-from-anonymous-statistics").write_text("")
    (root / "config/stream.conf").write_text("[stream]\n    enabled = no\n")


def request(port, target, method="GET", payload=None, compressed=False, read_timeout=30):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=read_timeout)
    headers = {"Connection": "close", "Accept-Encoding": "gzip" if compressed else "identity"}
    if payload is not None:
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    try:
        connection.request(method, target, body=payload, headers=headers)
        response = connection.getresponse()
        wire = response.read()
        result = {"status": response.status, "headers": dict(response.getheaders()), "wire_bytes": len(wire)}
        if response.getheader("Content-Length") is not None and int(response.getheader("Content-Length")) != len(wire):
            raise RuntimeError("HTTP Content-Length differs from delivered bytes")
        body = gzip.decompress(wire) if response.getheader("Content-Encoding") == "gzip" else wire
        result["body_bytes"] = len(body)
        return result, body
    finally:
        connection.close()


def target(endpoint, parameters, prefix=""):
    return prefix + "/api/v1/" + endpoint + "?" + urlencode(parameters, doseq=True)


def qualify(binary, codec, run):
    root = make_fixture(run, codec)
    configuration(root, "raw" if codec == "array" else "gorilla")
    environment = {key: value for key, value in os.environ.items() if not key.startswith("NETDATA_")}
    environment.update(PROMQL_PROTOTYPE_FIXTURE_ROOT=str(root), DO_NOT_TRACK="1")
    command = [str(binary), "-D", "-c", "config/netdata.conf", "-W", "promql-http-fixture"]
    print(f"{codec}: {shlex.join(command)}", flush=True)
    item = {"codec": codec, "command": command, "checks": [], "passed": False}
    ready = None
    with (root / "agent.stdout").open("wb") as stdout, (root / "agent.stderr").open("wb") as stderr:
        process = subprocess.Popen(command, cwd=root, env=environment, stdout=stdout, stderr=stderr)
        item["pid"] = process.pid
        (root / "process.json").write_text(json.dumps(item, indent=2) + "\n")
        try:
            deadline = time.monotonic() + 60
            ready_path = root / "cache/promql-live-ready.json"
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(f"fixture exited before readiness: {process.returncode}")
                if ready_path.exists():
                    try:
                        ready = json.loads(ready_path.read_text())
                    except json.JSONDecodeError:
                        pass
                    if ready is not None:
                        break
                time.sleep(0.05)
            if ready is None or ready["pid"] != process.pid:
                raise RuntimeError("the owned fixture did not publish readiness")
            port, base = ready["port"], ready["base"]
            info, body = request(port, "/api/v1/info")
            identity = json.loads(body)
            if info["status"] != 200 or identity.get("uid") != ready["host"] or "promql-fixture-main" not in identity.get("mirrored_hosts", []):
                raise RuntimeError("HTTP instance identity differs from the owned fixture")
            item["ready"] = ready

            def check(name, endpoint, parameters, expected=None, method="GET", payload=None, prefix="", status=200, compressed=False, inspect=None, read_timeout=30):
                observed, body = request(port, target(endpoint, parameters, prefix), method, payload, compressed, read_timeout)
                value = json.loads(body)
                record = {"name": name, **observed, "passed": False}
                item["checks"].append(record)
                if observed["status"] != status or (expected is not None and value != expected) or (inspect is not None and not inspect(value)):
                    raise RuntimeError(f"{name}: unexpected HTTP result: {body[:800]!r}")
                if compressed and observed["headers"].get("Content-Encoding") != "gzip":
                    raise RuntimeError(f"{name}: requested gzip response was not encoded")
                record["passed"] = True

            def scalar(value, when=base + 4):
                return {"status": "success", "data": {"resultType": "scalar", "result": [when, str(value)]}}

            local, child = "/host/" + ready["host"], "/host/" + ready["child"]
            gauge = 'sum(fixture_http{netdata_dimension_id="gauge"})'
            counter = 'scalar(fixture_http{netdata_dimension_id="counter"})'
            for name, scope, value in [("fleet-gauge", "", 12), ("local-gauge", local, 5), ("child-gauge", child, 7)]:
                check(name, "query", {"query": f"scalar({gauge})", "time": base + 4}, scalar(value), prefix=scope)
            average = 'sum(avg_over_time(fixture_http{netdata_dimension_id="gauge"}[3s]))'
            for name, scope, value in [("fleet-average", "", 12), ("local-average", local, 5), ("child-average", child, 7)]:
                check(name, "query", {"query": f"scalar({average})", "time": base + 4}, scalar(value), prefix=scope)
            check("fleet-average-range", "query_range", {"query": average, "start": base + 3, "end": base + 5, "step": 1},
                  {"status": "success", "data": {"resultType": "matrix", "result": [
                      {"metric": {}, "values": [[base + index, "12"] for index in range(3, 6)]}]}})
            buckets = ('label_replace(fixture_http{netdata_dimension_id="gauge"},"le","1","netdata_dimension_id","gauge") or '
                       'label_replace(fixture_http{netdata_dimension_id="gauge"},"le","+Inf","netdata_dimension_id","gauge")')
            quantile = f"sum(histogram_quantile(0.5,({buckets})))"
            for name, scope, value in [("fleet-histogram-quantile", "", 1), ("local-histogram-quantile", local, 0.5), ("child-histogram-quantile", child, 0.5)]:
                check(name, "query", {"query": f"scalar({quantile})", "time": base + 4}, scalar(value), prefix=scope)
            check("fleet-histogram-quantile-range", "query_range", {"query": quantile, "start": base + 3, "end": base + 5, "step": 1},
                  {"status": "success", "data": {"resultType": "matrix", "result": [
                      {"metric": {}, "values": [[base + index, "1"] for index in range(3, 6)]}]}})
            grouped_buckets = " or ".join(
                f'label_replace(({buckets}),"fixture_group","{group}","netdata_dimension_id","gauge")'
                for group in ["a", "b", "c"])
            grouped_quantile = f"sum(histogram_quantile(0.5,({grouped_buckets})))"
            for name, scope, value in [("fleet-grouped-histogram", "", 3), ("local-grouped-histogram", local, 1.5), ("child-grouped-histogram", child, 1.5)]:
                check(name, "query", {"query": f"scalar({grouped_quantile})", "time": base + 4}, scalar(value), prefix=scope)
            check("fleet-grouped-histogram-range", "query_range", {"query": grouped_quantile, "start": base + 3, "end": base + 5, "step": 1},
                  {"status": "success", "data": {"resultType": "matrix", "result": [
                      {"metric": {}, "values": [[base + index, "3"] for index in range(3, 6)]}]}}, read_timeout=180)
            check("context-name", "query", {"query": 'scalar(sum({__name__="fixture.http",netdata_dimension_id="gauge"}))', "time": base + 4}, scalar(12))
            check("collected-counter", "query", {"query": counter, "time": base + 4}, scalar(8), prefix=local)
            check("counter-reset", "query", {"query": counter, "time": base + 6}, scalar(0, base + 6), prefix=local)
            check("counter-rate", "query", {"query": 'scalar(rate(fixture_http{netdata_dimension_id="counter"}[3s]))', "time": base + 4}, prefix=local,
                  inspect=lambda v: {k: value for k, value in v.items() if k != "infos"} == scalar(2) and
                  len(v.get("infos", [])) == 1 and "metric might not be a counter" in v["infos"][0] and
                  "fixture_http" in v["infos"][0] and v["infos"][0].endswith("(1:13)"))
            check("form-post", "query", {"query": "2", "time": base + 4}, scalar(1), method="POST", payload="query=1")
            check("label-values", "label/netdata_units/values", {"match[]": "fixture_http"}, {"status": "success", "data": ["KiB"]})
            check("dimension-values", "label/netdata_dimension_id/values", {"match[]": "fixture_http"}, {"status": "success", "data": ["counter", "gauge"]})
            check("labels", "labels", {"match[]": "fixture_http"}, inspect=lambda v: v["status"] == "success" and {"__name__", "netdata_host_id", "netdata_units", "fixture_role"} <= set(v["data"]))
            for name, scope, count in [("fleet-series", "", 4), ("local-series", local, 2)]:
                check(name, "series", {"match[]": "fixture_http", "start": base, "end": base + 10}, prefix=scope,
                      inspect=lambda v, count=count: v["status"] == "success" and len(v["data"]) == count)
            for compressed in [False, True]:
                check("range-gzip" if compressed else "range-plain", "query_range", {"query": "vector(1)", "start": base, "end": base + 99999, "step": 1}, compressed=compressed,
                      inspect=lambda v: v["status"] == "success" and v["data"]["resultType"] == "matrix" and len(v["data"]["result"]) == 1 and
                      v["data"]["result"][0]["metric"] == {} and len(v["data"]["result"][0]["values"]) == 100000 and
                      all(point == [base + index, "1"] for index, point in enumerate(v["data"]["result"][0]["values"])))
            check("expired-timeout", "query", {"query": "1", "time": base, "timeout": "0"}, status=503,
                  inspect=lambda v: v["status"] == "error" and v["errorType"] == "timeout")
            check("parse-error", "query", {"query": "sum(", "time": base}, status=400,
                  inspect=lambda v: v["status"] == "error" and v["errorType"] == "bad_data")
            item["passed"] = True
        finally:
            if process.poll() is None and ready is not None and ready.get("pid") == process.pid:
                try:
                    item["shutdown"], _ = request(ready["port"], "/exit")
                except (OSError, http.client.HTTPException) as error:
                    item["shutdown_read_error"] = str(error)
            try:
                item["exit"] = process.wait(timeout=60)
                if item["exit"] != 0:
                    item["passed"] = False
                    raise RuntimeError(f"owned Agent exited with status {item['exit']}")
            finally:
                stdout.flush()
                rows = [json.loads(line) for line in (root / "agent.stdout").read_text().splitlines() if line.startswith("{")]
                item["host_readiness"] = rows
                if {row.get("host_readiness_case") for row in rows} != HOST_CASES or any(row.get("errors") for row in rows):
                    item["passed"] = False
                (root / "result.json").write_text(json.dumps(item, indent=2) + "\n")
    if not item["passed"]:
        raise RuntimeError("initial host metadata qualification failed")
    return item


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--codecs", nargs="+", choices=["array", "gorilla"], default=["array", "gorilla"])
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    parent = safe_parent(REPO / ".tmp")
    run = Path(tempfile.mkdtemp(prefix="live-api-", dir=parent)).resolve(strict=True)
    if run.parent != parent or run.stat().st_mode & 0o077:
        raise RuntimeError("invalid private fixture directory")
    receipt = {"binary_sha256": sha256(binary), "sources": source_binding(), "runs": [], "passed": False}
    receipt["sources"][str(Path(__file__).resolve().relative_to(REPO))] = sha256(Path(__file__).resolve())
    try:
        for codec in args.codecs:
            receipt["runs"].append(qualify(binary, codec, run))
        current = source_binding()
        current[str(Path(__file__).resolve().relative_to(REPO))] = sha256(Path(__file__).resolve())
        if current != receipt["sources"] or sha256(binary) != receipt["binary_sha256"]:
            raise RuntimeError("source or binary changed during qualification")
        receipt["passed"] = True
    finally:
        (run / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"receipt: {run / 'receipt.json'}", flush=True)


if __name__ == "__main__":
    main()
