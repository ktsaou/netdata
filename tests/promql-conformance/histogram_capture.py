#!/usr/bin/env python3
"""Capture source-derived histogram kernels in an unchanged pinned Go facade."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile

from build import directory, run
from capture import verify_facade
from upstream_inventory import inventory

HERE = Path(__file__).resolve().parent
COMMIT = "5241a27fe3c6983549fccc32f6e65917408c63cd"
MODULE = "github.com/prometheus/prometheus@v0.315.0"
HOOK = "model/histogram/netdata_capture_test.go"


def helper(table, index):
    mode = table["mode"]
    if mode == "float-equality-pair":
        source = table["prefix"]
        converted = table["test"] == "TestHistogramEquals"
        histogram = "Histogram" if converted else "FloatHistogram"
        closures = [("equals", True), ("notEquals", False)]
        if converted:
            closures.append(("notEqualsUntilFloatConv", True))
        for name, expected in closures:
            anchor = f'{name} := func(h1, h2 {histogram}) {{'
            if source.count(anchor) != 1:
                raise ValueError("changed procedural equality assertion closure")
            arguments = 'h1f, h2f' if converted else '&h1, &h2'
            observation = (f'\nnetdataEqualityRecord(output, fmt.Sprintf("{table["test"]}:%03d", checkpoint), '
                           f'{arguments}, {str(expected).lower()})\ncheckpoint++\n')
            end = source.index('\n\t}', source.index(anchor) + len(anchor))
            source = source[:end] + observation + source[end:]
        return (f'func netdataTable{index}(t *testing.T, output *[]map[string]any) {{\n'
                f'checkpoint := 0\n{source}\n'
                f'if checkpoint != {table["cases"]} {{ t.Fatal("changed equality observation count") }}\n}}')
    body = [f'func netdataTable{index}(t *testing.T, output *[]map[string]any) {{', table["prefix"]]
    singleton = table["variable"] in {"h", "c"}
    if singleton:
        body += [f'id := "{table["test"]}:000"']
    elif table["case_keys"] is not None:
        keys = ", ".join(json.dumps(key) for key in sorted(table["case_keys"]))
        body += [f'for _, name := range []string{{{keys}}} {{', f'c := {table["variable"]}[name]',
                 f'id := "{table["test"]}:" + name']
    else:
        body += [f'for i, c := range {table["variable"]} {{', f'id := fmt.Sprintf("{table["test"]}:%03d", i)']
    if mode in {"float-mul", "float-div"}:
        input_field, factor, operation = (("in", "scale", "Mul") if mode == "float-mul"
                                          else ("fh", "s", "Div"))
        body += [f'request := netdataFrozen(map[string]any{{"input": netdataFloat(c.{input_field}),',
                 f'"scalar": fmt.Sprintf("%016x", math.Float64bits(c.{factor}))}})',
                 'declared := netdataFrozen(netdataFloat(c.expected))',
                 f'actual := c.{input_field}.{operation}(c.{factor})',
                 f'netdataRecord(output, id, "{mode}", request, netdataFloat(actual), declared)']
    elif mode in {"float-copy", "float-copy-to", "integer-copy", "integer-copy-to"}:
        encoder, histogram = (("netdataFloat", "FloatHistogram") if mode.startswith("float")
                              else ("netdataInteger", "Histogram"))
        body += [f'request := netdataFrozen(map[string]any{{"input": {encoder}(c.orig), "destination": nil}})',
                 f'declared := netdataFrozen({encoder}(c.expected))']
        if mode.endswith("-to"):
            body += [f'actual := &{histogram}{{}}', 'c.orig.CopyTo(actual)']
        else:
            body += ['actual := c.orig.Copy()']
        body += [f'netdataRecord(output, id, "{mode}", request, {encoder}(actual), declared)']
    elif mode == "integer-to-float":
        body += ['request := netdataFrozen(map[string]any{"input": netdataInteger(&h),',
                 '"destination": netdataFloat(c.fh)})', 'actual := h.ToFloat(c.fh)',
                 'netdataRecord(output, id, "integer-to-float", request, netdataFloat(actual), nil)']
    elif mode == "float-compact":
        body += ['request := netdataFrozen(map[string]any{"input": netdataFloat(c.in), "max_empty": c.maxEmptyBuckets})',
                 'declared := netdataFrozen(netdataFloat(c.expected))', 'actual := c.in.Compact(c.maxEmptyBuckets)',
                 'netdataRecord(output, id, "float-compact", request, netdataFloat(actual), declared)']
    elif mode == "float-copy-to-schema":
        body += ['request := netdataFrozen(map[string]any{"input": netdataFloat(c.in), "target_schema": c.targetSchema})',
                 'declared := netdataFrozen(map[string]any{"result": netdataFloat(c.expected), "input_after": netdataFloat(c.in)})',
                 'actual := c.in.CopyToSchema(c.targetSchema)',
                 'netdataRecord(output, id, "float-copy-to-schema", request, map[string]any{"result": netdataFloat(actual), "input_after": netdataFloat(c.in)}, declared)']
    elif mode == "float-reduce-resolution":
        body += ['request := netdataFrozen(map[string]any{"input": netdataFloat(c.origin), "target_schema": c.targetSchema})',
                 'declared := netdataFrozen(map[string]any{"result": netdataFloat(c.target), "error": c.errorMsg, "histogram_error": c.errorMsg != ""})',
                 'err := c.origin.ReduceResolution(c.targetSchema)',
                 'netdataRecord(output, id, "float-reduce-resolution", request, netdataReductionResult(c.origin, err), declared)']
    elif mode == "exponential-bound":
        body += ['request := map[string]any{"index": c.idx, "schema": c.schema}',
                 'declared := fmt.Sprintf("%016x", math.Float64bits(c.want))',
                 'actual := fmt.Sprintf("%016x", math.Float64bits(getBoundExponential(c.idx, c.schema)))',
                 'netdataRecord(output, id, "exponential-bound", request, actual, declared)']
    elif mode == "float-iterator-set":
        test = table["test"]
        start = 'float64(0)'
        histogram = 'h' if singleton else '&c.h'
        target = 'h.Schema' if singleton else 'c.h.Schema'
        if test in {"TestAllFloatBucketIterator", "TestAllReverseFloatBucketIterator"}:
            reverse = test == "TestAllReverseFloatBucketIterator"
            route = "all-reverse" if reverse else "all"
            body += [f'routes := []string{{"{route}"}}',
                     f'declared := netdataIteratorDeclared(map[string][]Bucket[float64]{{"{route}": netdataDeclaredAll(&c.h, c.includeNeg, c.includeZero, c.includePos, {str(reverse).lower()})}})']
        elif test == "TestFloatBucketIteratorTargetSchema":
            target = 'int32(-1)'
            body += ['routes := []string{"low-positive", "low-negative"}',
                     'declared := netdataIteratorDeclared(map[string][]Bucket[float64]{"low-positive": c.expPositiveBuckets, "low-negative": c.expNegativeBuckets})']
        elif test == "TestFloatCustomBucketsIterators":
            body += ['routes := netdataIteratorRoutes()',
                     'positive := c.expPositiveBuckets', 'reverse := netdataReverseBuckets(positive)',
                     'declared := netdataIteratorDeclared(map[string][]Bucket[float64]{"all": positive, "all-reverse": reverse, "positive": positive, "negative": nil, "positive-reverse": reverse, "negative-reverse": nil, "low-positive": positive, "low-negative": nil})']
        elif test == "TestReverseFloatBucketIterator":
            body += ['routes := []string{"positive", "positive-reverse", "negative", "negative-reverse"}',
                     'positive := netdataCollectBuckets(h.PositiveBucketIterator())',
                     'negative := netdataCollectBuckets(h.NegativeBucketIterator())',
                     'declared := netdataIteratorDeclared(map[string][]Bucket[float64]{"positive": positive, "positive-reverse": netdataReverseBuckets(positive), "negative": negative, "negative-reverse": netdataReverseBuckets(negative)})']
        else:
            raise ValueError("unknown histogram iterator source table")
        body += [f'request := netdataFrozen(map[string]any{{"input": netdataFloat({histogram}), "routes": routes, "start": fmt.Sprintf("%016x", math.Float64bits({start})), "target_schema": {target}}})',
                 f'actual := netdataIteratorResult({histogram}, routes, {start}, {target})',
                 'netdataRecord(output, id, "float-iterator-set", request, actual, declared)']
    elif mode == "float-detect-reset":
        body += ['request := netdataFrozen(map[string]any{"current": netdataFloat(c.current), "previous": netdataFloat(c.previous)})',
                 'declared := netdataFrozen(map[string]any{"reset": c.resetExpected, "current_after": netdataFloat(c.current), "previous_after": netdataFloat(c.previous)})',
                 'actual := netdataResetResult(c.current, c.previous)',
                 'netdataRecord(output, id, "float-detect-reset", request, actual, declared)']
    elif mode == "custom-bounds-match":
        body += ['request := map[string]any{"left": netdataNumbers(c.c1), "right": netdataNumbers(c.c2)}',
                 'declared := map[string]any{"forward": c.want, "reverse": c.want}',
                 'actual := map[string]any{"forward": CustomBucketBoundsMatch(c.c1, c.c2), "reverse": CustomBucketBoundsMatch(c.c2, c.c1)}',
                 'netdataRecord(output, id, "custom-bounds-match", request, actual, declared)']
    elif mode == "float-arithmetic-set":
        test = table["test"]
        expected = 'c.expectedSum' if test == "TestKahanAddWithCompHistogram" else 'c.expected'
        body += ['requests, actuals, declarations := []any{}, []any{}, []any{}',
                 'capture := func(operation string, left, right, correction, expected *FloatHistogram) {',
                 'request, actual := netdataArithmeticPair(operation, left, right, correction, expected != nil)',
                 'var final any', 'if expected != nil { final = netdataFloat(expected.Copy().Compact(0)) }',
                 'var flags any', 'if operation != "sub" || expected != nil {',
                 'flags = map[string]any{"counter_reset_collision": c.expCounterResetCollision, "nhcb_bounds_reconciled": c.expNHCBBoundsReconciled}', '}',
                 'requests = append(requests, request)', 'actuals = append(actuals, actual)',
                 'declarations = append(declarations, netdataFrozen(map[string]any{"error": c.expErrMsg, "flags": flags, "final": final}))', '}']
        if test == "TestFloatHistogramAdd":
            body += [f'capture("{operation}", {left}, {right}, nil, {expected})'
                     for operation, left, right in (("add", "c.in1", "c.in2"), ("add", "c.in2", "c.in1"),
                                                   ("kahan", "c.in1", "c.in2"), ("kahan", "c.in2", "c.in1"))]
        elif test == "TestKahanAddWithCompHistogram":
            body += ['capture("kahan", c.in1, c.in2, c.comp, c.expectedSum)']
        elif test == "TestFloatHistogramSub":
            body += ['capture("sub", c.in1, c.in2, nil, c.expected)', 'var negative *FloatHistogram',
                     'if c.expected != nil { negative = c.expected.Copy().Mul(-1); negative.CounterResetHint = c.expected.CounterResetHint }',
                     'capture("sub", c.in2, c.in1, nil, negative)']
        else:
            raise ValueError("unknown histogram arithmetic source table")
        body += ['netdataRecord(output, id, "float-arithmetic-set", map[string]any{"operations": requests},',
                 'map[string]any{"operations": actuals}, map[string]any{"operations": declarations})']
    elif mode == "float-kahan-compact":
        body += ['request := netdataFrozen(map[string]any{"input": netdataFloat(h), "compensation": netdataFloat(c), "max_empty": 3})',
                 'declared := map[string]any{"positive_spans": []Span{{0, 5}},',
                 '"positive_buckets": netdataNumbers([]float64{1, 0, 2, 0, 3}), "compensation_positive_buckets": netdataNumbers([]float64{0.1, 0, 0.2, 0, 0.3})}',
                 'h, c = h.kahanCompact(3, c)',
                 'netdataRecord(output, id, "float-kahan-compact", request, map[string]any{"input_after": netdataFloat(h), "compensation_after": netdataFloat(c)}, declared)']
    else:
        raise ValueError("unknown source histogram table")
    body += ['}'] if singleton else ['}', '}']
    return "\n".join(body)


def generate_hook(tables):
    template = (HERE / "capture/histogram_test.go.in").read_text()
    original = "\n".join(f'{{"{row["test"]}", {row["test"]}}},' for row in tables)
    calls = "\n".join(f'netdataTable{i}(t, &cases)' for i in range(len(tables)))
    helpers = "\n\n".join(helper(row, i) for i, row in enumerate(tables))
    for marker, replacement in (("ORIGINAL_ASSERTIONS", original), ("CAPTURE_CALLS", calls),
                                ("CAPTURE_HELPERS", helpers)):
        marker = "/* " + marker + " */"
        if template.count(marker) != 1:
            raise ValueError("changed histogram hook marker")
        template = template.replace(marker, replacement)
    return template.encode()


def capture(mod_cache, write_input_ledger=False):
    cache = directory(mod_cache)
    source = directory(cache / MODULE)
    inventory(source)
    parent = HERE / ".tmp"
    parent.mkdir(exist_ok=True)
    directory(parent)
    task = directory(tempfile.mkdtemp(prefix="histogram-capture-", dir=parent))
    facade = task / "reference"
    private_tmp = task / "tmp"
    private_tmp.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(GOMODCACHE=str(cache), GOCACHE=str(parent / "go-build"), GOPROXY="off", GOSUMDB="off",
               GOTOOLCHAIN="local", GOWORK="off", GOFLAGS="-mod=readonly",
               TMPDIR=str(directory(private_tmp)), GOTMPDIR=str(private_tmp))
    run(["go", "mod", "verify"], HERE / "oracle", env)
    shutil.copytree(source, facade)
    directory(facade)
    for path in [facade, *facade.rglob("*")]:
        if path.is_dir():
            path.chmod(path.stat().st_mode | 0o200)
    manifest = verify_facade(source, facade)
    command = ["go", "run", HERE / "capture/histogram_tables.go", facade / "model/histogram"]
    print(f"{HERE / 'oracle'} > {shlex.join(map(str, command))}", flush=True)
    result = subprocess.run(list(map(str, command)), cwd=HERE / "oracle", env=env,
                            capture_output=True, text=True, check=False)
    if result.returncode:
        raise SystemExit(f"histogram table extraction failed ({result.returncode}): {result.stderr}")
    tables = json.loads(result.stdout)
    if len(tables) != 25 or sum(row["cases"] for row in tables) != 238:
        raise ValueError("changed histogram table coverage")
    for table in tables:
        raw = (source / "model/histogram" / table["file"]).read_bytes()
        if hashlib.sha256(raw).hexdigest() != table["source_sha256"]:
            raise ValueError("histogram table source attribution changed")
    hook = generate_hook(tables)
    destination = facade / HOOK
    if destination.exists():
        raise ValueError("histogram hook would replace reference source")
    destination.write_bytes(hook)
    env["NETDATA_HISTOGRAM_CAPTURE"] = str(task / "histograms.json")
    run(["go", "test", "-p", "4", "-count=1", "-timeout", "10m", "-run",
         "^(TestNetdataCaptureHistogram|TestIsInf)$", "./model/histogram", "./util/kahansum"], facade, env)
    if verify_facade(source, facade) != manifest or destination.read_bytes() != hook:
        raise ValueError("histogram source or hook changed during capture")
    files = {path.relative_to(facade).as_posix() for path in facade.rglob("*") if path.is_file()}
    if files != manifest.keys() | {HOOK} or any(path.is_symlink() for path in facade.rglob("*")):
        raise ValueError("unlisted histogram facade source")
    output = (task / "histograms.json").read_bytes()
    from histogram_reference import input_ledger, validate_cases
    data = json.loads(output)
    if write_input_ledger:
        digest = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        if digest != input_ledger()["source_manifest_sha256"]:
            raise ValueError("source pin changed during input-ledger generation")
        ledger = {"schema_version": 1, "commit": COMMIT, "module": MODULE,
                  "source_manifest_sha256": digest, "tables": tables,
                  "cases": [{key: case[key] for key in ("id", "mode", "request", "declared")} for case in data["cases"]]}
        counts = validate_cases(data, tables, ledger=ledger)
        (HERE / "histogram-original-inputs.json").write_text(json.dumps(ledger, indent=2) + "\n")
    else:
        counts = validate_cases(data, tables)
    (task / "tables.json").write_text(json.dumps(tables, indent=2) + "\n")
    (task / "source-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    metadata = {"schema_version": 1, "commit": COMMIT, "source_files": len(manifest),
                "hook_sha256": hashlib.sha256(hook).hexdigest(),
                "generator_sha256": hashlib.sha256((HERE / "capture/histogram_tables.go").read_bytes()).hexdigest(),
                "template_sha256": hashlib.sha256((HERE / "capture/histogram_test.go.in").read_bytes()).hexdigest(),
                "tables_sha256": hashlib.sha256((task / "tables.json").read_bytes()).hexdigest(),
                "output_sha256": hashlib.sha256(output).hexdigest(), "counts": counts}
    (task / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    print(json.dumps({"capture": str(task), "counts": counts}), flush=True)
    return task


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mod-cache", required=True, help="existing pinned module cache; downloads disabled")
    parser.add_argument("--write-input-ledger", action="store_true", help="explicitly regenerate public input bindings from verified Go execution")
    args = parser.parse_args()
    capture(args.mod_cache, args.write_input_ledger)
