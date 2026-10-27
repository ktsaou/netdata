#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Qualify real isolated counter storage; retain every process log and storage file."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
FAULT_CASES = {
    "warm-success", "cancel-before-open", "refuse-open", "refuse-partial-page", "refuse-warm-view",
    "cancel-point", "cold-success", "refuse-cold-routing", "refuse-worker-view", "cancel-worker-view",
    "private-collision-success", "private-refusal-precedes-cancel",
}
COLLECTION_CASES = {(name, tier) for name in [
    "mixed-ordinary", "mixed-counter", "negative-extrema", "partial-gap",
] for tier in range(1, 5)} | {
    ("all-gap-reset", 1), ("after-gap-reset", 1), ("mixed-gauge-start", 0), ("mixed-counter-start", 0),
    ("raw-counter", 0), ("raw-reset", 0), ("raw-new-cadence", 0),
    ("ordinary-backfill", 1), ("coarse-backfill", 2),
}
PROVIDER_CASES = {
    "legacy-start", "anchored-start", "alias-common-basis", "aliases-logical", "empty-grid", "reset",
    "positive-counter", "legacy-negative", "legacy-absolute", "gauge", "reserved-chart-labels", "backed-quantile",
    "gap-prefix", "reset-gap-prefix", "fine-coarse-fine", "fine-coarse-first", "fine-coarse-last",
    "fine-coarse-negative", "fine-coarse-absolute", "fine-coarse-native", "fine-coarse-alias", "fine-coarse-replay",
}
PROVIDER_CASES |= {f"checkpoint-{mode}-{pages}" for mode in ("native", "legacy")
                   for pages in (16, 17, 33, 129, 513, 2049)}
PROVIDER_CASES |= {"checkpoint-short-no-backing", "checkpoint-long-no-backing"}
PROVIDER_CASES |= {"checkpoint-reset-gap", "checkpoint-reset-gap-alias", "checkpoint-negative", "checkpoint-absolute"}
CHECKPOINT_CALLBACK_CASES = {"native-seeks", "legacy-seeks", "alias-basis", "failed-open-cleanup",
                             "failed-seek-sticky", "two-metrics-one-file", "native-cold-seek",
                             "legacy-cold-seek", "reset-gap-cold-seek", "native-late-open", "legacy-late-open",
                             "reset-gap-late-open", "mixed-late-open"}
DISCOVERY_CASES = {"negative", "reversed", "between-seconds", "zero", "positive-wide", "full-clock", "future-wide"}
OUTPUT_CASES = {"scalar-no-backing", "string-escaping", "vector-no-backing", "large-no-backing",
                "plain-2049", "plain-100000", "gzip-2049", "gzip-100000"}
NATIVE_CASES = {"plain-2049", "plain-100000", "gzip-2049", "gzip-100000"}
METADATA_REF_CASES = {"immutable-source-views", "archived-cold", "archived-hit", "names-one-context", "names-archived-unloaded", "names-first-witness",
                      "names-later-tier-witness", "names-retention-hole", "names-no-retention"}
REQUEST_CASES = {"body-first", "escaped-separators", "embedded-nul", "empty-first", "invalid-field", "semicolon-order",
                 "escaped-semicolon", "non-form-body", "mime-error-first", "mime-duplicate-same",
                 "mime-duplicate-different", "body-error-before-url", "mime-unicode-i", "mime-unicode-k"}
DESCRIPTION_CASES = {"archived-description-unloaded", "description-no-samples", "description-frozen-strings",
                     "description-updated-strings", "description-deleted-metric", "description-deleted-instance",
                     "description-deleted-context", "description-no-dimensions", "description-cancel-after-row"}
DESCRIPTION_SORT_CASES = {"empty", "single", "sorted-long", "reverse-long", "cancel-initialization",
                          "cancel-comparison", "cancel-emission", "read-failure", "zero-limit-long"}
API_HANDLER_CASES = {
    "scalar", "live-gauge", "counter-absolute", "counter-legacy", "fleet-aliases", "form-body-priority", "range",
    "range-end-order", "range-zero-step", "range-type", "range-submillisecond", "empty-query", "time-wrapper",
    "limit-before-time", "timeout-before-lookback", "lookback-unwrapped", "expired-timeout", "expired-parse-first",
    "stats-warning", "reject-shaping", "label-values", "label-unescaped", "label-name-before-limit", "label-nul-rune",
    "label-get-only", "labels-limit", "values-union", "values-sorted", "series-requires-match",
    "series-limit-before-match", "series-form-error-first", "series-empty-guard", "labels-empty-guard",
    "selector-syntax-before-guard", "discovery-reversed", "execution-error-drops-infos",
    "caller-expired", "caller-expired-before-parse", "caller-keeps-uri-deadline", "cancel-on-registration",
    "pooled-release",
    "metadata-alias", "metadata-exact", "metadata-missing", "metadata-zero", "metadata-negative", "metadata-one", "metadata-per-zero", "metadata-ignore-time", "metadata-max", "metadata-min", "metadata-plus", "metadata-duplicate", "metadata-overflow", "metadata-underflow", "metadata-invalid", "metadata-space", "metadata-nul", "metadata-per-invalid",
    "metadata-get-only", "metadata-cancel", "metadata-zero-unfiltered", "metadata-zero-per-invalid", "metadata-zero-cancel",
}
MQTT_SOURCE_CASES = {"short-8193", "short-1000000", "timeout-release", "reconnect-purge", "read-error-stops",
                     "compact-partial", "view-refusal", "wire-limits"}
ACLK_SOURCE_CASES = {"plain-8193", "plain-1000000", "gzip-1000000", "long-prefix", "no-prefix-backing"}
RTC_FRAME_CASES = {"plain-100000", "negotiated-2049", "unicode-205", "unicode-too-small", "stopped-before-send", "send-failure"}
OPERATION_CASES = {"socket-close", "retired-id-same-fd", "caller-deadline-first", "uri-deadline-first",
                   "cancel-before-activation", "watcher-retirement", "shutdown-admission"}
API_HANDLER_CASES |= {'names-bounds', 'names-future', 'names-subsecond', 'names-negative', 'names-mixed-union', 'names-exact', 'names-fallback', 'names-reversed', 'names-conjunction-empty', 'names-fallback-empty', 'names-limit', 'names-union', 'names-regex', 'names-original'}
API_ROUTE_CASES = API_HANDLER_CASES - {"pooled-release"}
RTC_PEER_CASES = {"connect", "live-gauge", "form-post", "paced-range", "remote-close", "shutdown"}


def sha256(path):
    value = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def source_binding():
    sources = sorted((REPO / "src/database/engine").glob("*"))
    sources += sorted((REPO / "src/web/api/promql").glob("*"))
    sources += sorted((REPO / "src/database/contexts").glob("*"))
    library = REPO / "src/crates/promql-engine"
    sources += sorted((library / "src").rglob("*.rs"))
    sources += [library / "Cargo.toml", library / "Cargo.lock", library / "include/netdata-promql.h"]
    sources += [REPO / name for name in [
        "src/daemon/main.c", "src/daemon/daemon-shutdown.c", "src/database/storage-engine.h", "src/database/rrddim-collection.c",
        "src/database/sqlite/sqlite_metadata.c", "src/database/sqlite/sqlite_aclk.c",
        "src/database/rrddim-collection.h", "src/database/rrddim.c", "src/database/rrddim.h",
        "src/database/rrddim-backfill.c", "src/database/rrddim-backfill.h",
        "src/database/rrdset-collection.c", "src/database/rrdset-collection.h",
        "src/database/rrdset-index-id.c", "src/database/rrdset-index-id.h",
        "src/database/rrdset-index-name.c", "src/database/rrdhost.c", "src/database/rrdhost.h",
        "src/libnetdata/storage-point.h", "src/libnetdata/storage_number/storage_number.c",
        "src/libnetdata/storage_number/storage_number.h", "src/libnetdata/aral/aral.c",
        "packaging/cmake/Modules/NetdataSourceLists.cmake", "tests/promql-integration/storage.py",
        "packaging/cmake/Modules/NetdataDaemon.cmake", "src/database/rrdlabels.c", "src/database/rrdlabels.h",
        "src/web/server/web_client.c", "src/web/server/web_client.h", "src/web/api/http_header.c",
        "src/web/api/web_api.c", "src/web/api/web_api_v1.c", "src/web/api/web_api.h",
        "src/web/server/web_client_cache.c", "src/web/server/web_client_cache.h",
        "src/libnetdata/query_progress/progress.c", "src/libnetdata/query_progress/progress.h",
        "src/database/sqlite/sqlite_context.c", "src/database/sqlite/sqlite_context.h",
        "src/aclk/mqtt_websockets/mqtt_ng.c", "src/aclk/mqtt_websockets/mqtt_ng.h",
        "src/aclk/mqtt_websockets/mqtt_wss_client.c", "src/aclk/mqtt_websockets/mqtt_wss_client.h",
        "src/aclk/mqtt_websockets/common_public.h", "src/aclk/mqtt_websockets/ws_client.c",
        "src/aclk/aclk_query.c", "src/aclk/aclk_tx_msgs.c", "src/aclk/aclk_tx_msgs.h",
        "src/web/rtc/webrtc.c", "src/web/rtc/webrtc.h",
        "src/web/rtc/webrtc-test.c",
    ]]
    return {str(path.relative_to(REPO)): sha256(path) for path in sources
            if path.is_file() and path.suffix in {".c", ".h", ".cc", ".cmake", ".py", ".rs", ".toml", ".lock"}}


def safe_parent(path):
    value = path.resolve(strict=True)
    if not value.is_dir() or len(value.parts) < 4 or len(str(value)) < 10:
        raise RuntimeError(f"unsafe fixture parent: {path}")
    return value


def make_fixture(parent, codec):
    root = parent / codec
    root.mkdir(mode=0o700)
    root = safe_parent(root)
    if root.parent != parent or root.stat().st_uid != os.getuid() or root.stat().st_mode & 0o077:
        raise RuntimeError("fixture ownership or permissions differ from the required private directory")
    names = ["cache", "lib", "log", "config", "stock", "web", "plugins"]
    for name in names:
        (root / name).mkdir(mode=0o700)
    for tier in range(5):
        (root / "cache" / f"storage-tier{tier}").mkdir(mode=0o700)
    (root / ".promql-prototype-fixture").write_text("isolated-promql-prototype\n")
    directories = [("cache", "cache"), ("lib", "lib"), ("log", "log"), ("config", "config"),
                   ("stock config", "stock"), ("stock data", "stock"), ("web", "web"), ("plugins", "plugins")]
    config = "[directories]\n" + "".join(f"    {key} = {safe_parent(root / name)}\n" for key, name in directories)
    config += "[db]\n    mode = ram\n    storage tiers = 1\n    dbengine page cache size = 16MiB\n    dbengine extent cache size = 16MiB\n"
    config += "[health]\n    enabled = no\n[registry]\n    enabled = no\n[cloud]\n    enabled = no\n[global]\n    libuv worker threads = 16\n"
    (root / "config/netdata.conf").write_text(config)
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Existing DBengine-enabled Agent build")
    parser.add_argument("--codecs", nargs="+", choices=["array", "gorilla"], default=["array", "gorilla"])
    parser.add_argument("--rtc-peers", action="store_true", help="Exercise actual loopback WebRTC peers in an enabled internal-check build")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError("Agent binary is not executable")
    parent = safe_parent(REPO / ".tmp")
    run = Path(tempfile.mkdtemp(prefix="dbengine-storage-", dir=parent)).resolve(strict=True)
    if run.parent != parent or run.stat().st_mode & 0o077:
        raise RuntimeError("unsafe run directory")
    initial_sources, binary_hash = source_binding(), sha256(binary)
    revision = subprocess.check_output(["git", "-C", str(REPO), "rev-parse", "HEAD"], text=True).strip()
    receipt = {"sources": initial_sources, "binary_sha256": binary_hash, "source_commit": revision, "phases": [], "passed": False,
               "scope": "real five-tier storage, contexts bridge, native handlers/dispatcher and bounded delivery components; "
                        + ("actual loopback WebRTC peers; " if args.rtc_peers else "controlled WebRTC framing; ")
                        + "no external HTTP or live Cloud connection"}
    receipt_path = run / "receipt.json"
    try:
        for codec in args.codecs:
            root = make_fixture(run, codec)
            for phase in ["write", "read", "append"]:
                environment = os.environ.copy()
                environment.update(PROMQL_PROTOTYPE_FIXTURE_ROOT=str(root),
                                   PROMQL_STORAGE_FIXTURE_PHASE=phase, PROMQL_STORAGE_FIXTURE_CODEC=codec)
                environment["PROMQL_RTC_PEER_FIXTURE"] = "1" if args.rtc_peers else "0"
                command = [str(binary), "-c", "config/netdata.conf", "-W", "promql-dbengine-storage"]
                print(f"{root.name}/{phase}: {shlex.join(command)}", flush=True)
                start = time.monotonic()
                with (root / f"{phase}.stdout").open("wb") as stdout, (root / f"{phase}.stderr").open("wb") as stderr:
                    process = subprocess.Popen(command, cwd=root, env=environment, stdout=stdout, stderr=stderr)
                    (root / f"{phase}.process.json").write_text(json.dumps({"pid": process.pid, "command": command,
                                                                          "binary_sha256": binary_hash}, indent=2) + "\n")
                    result = process.wait()
                rows = [json.loads(line) for line in (root / f"{phase}.stdout").read_text().splitlines()
                        if line.startswith("{")]
                item = {"codec": codec, "phase": phase, "exit": result, "seconds": time.monotonic() - start,
                        "records": rows, "stdout_sha256": sha256(root / f"{phase}.stdout"),
                        "stderr_sha256": sha256(root / f"{phase}.stderr")}
                receipt["phases"].append(item)
                receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
                geometry = [row for row in rows if "storage_geometry" in row]
                expected_geometry = {"singleton-intervals"} if phase == "read" else set()
                singletons = [row for row in rows if "storage_singleton" in row]
                expected_singletons = {(kind + "-" + cache, tier) for kind in ["known", "default"]
                                       for cache in ["cold", "warm"] for tier in range(5)} if phase == "read" else set()
                if phase == "read":
                    expected_singletons |= {(kind + "-" + cache, 0) for kind in ["known", "default"]
                                            for cache in ["journal", "journal-warm"]}
                faults = [row for row in rows if "storage_fault" in row]
                collections = [row for row in rows if "collection_case" in row]
                providers = [row for row in rows if "provider_case" in row]
                checkpoint_memory = [row for row in rows if "checkpoint_memory" in row]
                checkpoint_callbacks = [row for row in rows if "checkpoint_callback" in row]
                discoveries = [row for row in rows if "discovery_case" in row]
                outputs = [row for row in rows if "output_case" in row]
                natives = [row for row in rows if "native_case" in row]
                metadata_refs = [row for row in rows if "metadata_ref_case" in row]
                descriptions = [row for row in rows if "metadata_description_case" in row]
                description_sorts = [row for row in rows if "metadata_sort_case" in row]
                requests = [row for row in rows if "request_case" in row]
                api_handlers = [row for row in rows if "api_handler_case" in row]
                mqtt_sources = [row for row in rows if "mqtt_source_case" in row]
                aclk_sources = [row for row in rows if "aclk_source_case" in row]
                rtc_frames = [row for row in rows if "rtc_frame_case" in row]
                operations = [row for row in rows if "operation_case" in row]
                api_routes = [row for row in rows if "api_route_case" in row]
                rtc_peers = [row for row in rows if "rtc_peer_case" in row]
                expected_faults = FAULT_CASES if phase == "read" else set()
                expected_collections = COLLECTION_CASES if phase == "read" else set()
                expected_providers = PROVIDER_CASES if phase == "read" else set()
                expected_checkpoints = {"native", "legacy"} if phase == "read" else set()
                expected_checkpoint_callbacks = CHECKPOINT_CALLBACK_CASES if phase == "read" else set()
                expected_discoveries = DISCOVERY_CASES if phase == "read" else set()
                expected_outputs = OUTPUT_CASES if phase == "read" else set()
                expected_natives = NATIVE_CASES if phase == "read" else set()
                expected_refs = METADATA_REF_CASES if phase == "read" else set()
                expected_descriptions = DESCRIPTION_CASES if phase == "read" else set()
                expected_description_sorts = DESCRIPTION_SORT_CASES if phase == "read" else set()
                expected_requests = REQUEST_CASES if phase == "read" else set()
                expected_api = API_HANDLER_CASES if phase == "read" else set()
                expected_mqtt = MQTT_SOURCE_CASES if phase == "read" else set()
                expected_aclk = ACLK_SOURCE_CASES if phase == "read" else set()
                expected_rtc = RTC_FRAME_CASES if phase == "read" else set()
                expected_operations = OPERATION_CASES if phase == "read" else set()
                expected_routes = API_ROUTE_CASES if phase == "read" else set()
                expected_peers = RTC_PEER_CASES if phase == "read" and args.rtc_peers else set()
                if (result or len(rows) != 36 + len(expected_geometry) + len(expected_singletons) + len(expected_faults) + len(expected_collections) + len(expected_providers) + len(expected_checkpoints) + len(expected_checkpoint_callbacks) + len(expected_discoveries) + len(expected_outputs) + len(expected_natives) + len(expected_refs) + len(expected_descriptions) + len(expected_description_sorts) + len(expected_requests) + len(expected_api) + len(expected_mqtt) + len(expected_aclk) + len(expected_rtc) + len(expected_operations) + len(expected_routes) + len(expected_peers)
                        or {row["storage_geometry"] for row in geometry} != expected_geometry
                        or any(row["errors"] for row in rows)
                        or {(row["storage_singleton"], row["tier"]) for row in singletons} != expected_singletons
                        or {row["storage_fault"] for row in faults} != expected_faults
                        or {(row["collection_case"], row["tier"]) for row in collections} != expected_collections
                        or {row["provider_case"] for row in providers} != expected_providers
                        or {row["checkpoint_memory"] for row in checkpoint_memory} != expected_checkpoints
                        or {row["checkpoint_callback"] for row in checkpoint_callbacks} != expected_checkpoint_callbacks
                        or {row["discovery_case"] for row in discoveries} != expected_discoveries
                        or {row["output_case"] for row in outputs} != expected_outputs
                        or {row["native_case"] for row in natives} != expected_natives
                        or any(not row["short_write"] or row["bytes"] != row["received"] for row in natives)
                        or {row["metadata_ref_case"] for row in metadata_refs} != expected_refs
                        or {row["metadata_description_case"] for row in descriptions} != expected_descriptions
                        or {row["metadata_sort_case"] for row in description_sorts} != expected_description_sorts
                        or any(row["rows"] and not row["pointer_checks"] for row in descriptions)
                        or {row["request_case"] for row in requests} != expected_requests
                        or {row["api_handler_case"] for row in api_handlers} != expected_api
                        or {row["mqtt_source_case"] for row in mqtt_sources} != expected_mqtt
                        or {row["aclk_source_case"] for row in aclk_sources} != expected_aclk
                        or {row["rtc_frame_case"] for row in rtc_frames} != expected_rtc
                        or {row["operation_case"] for row in operations} != expected_operations
                        or {row["api_route_case"] for row in api_routes} != expected_routes
                        or {row["rtc_peer_case"] for row in rtc_peers} != expected_peers
                        or any(row["releases"] != 1 for row in mqtt_sources)
                        or any((row["rows"] and not row["pointer_checks"]) or row["remaining_provider"] or row["remaining_views"] for row in metadata_refs)
                        or any(row["remaining_handles"] for row in faults)):
                    raise RuntimeError(f"{codec}/{phase} failed; retained logs in {root}")
        if source_binding() != initial_sources or sha256(binary) != binary_hash:
            raise RuntimeError("source or binary changed during qualification")
        receipt["passed"] = True
    finally:
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
        print(f"receipt: {receipt_path}", flush=True)


if __name__ == "__main__":
    main()
