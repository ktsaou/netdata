# Install and try the PromQL branch

This guide builds the `feature/promql-rust-core` branch, installs a private Linux Agent, and runs PromQL against
data collected on that machine. It uses a separate directory and a loopback HTTP port. Run it as an ordinary user.

## What has been implemented

- An independent Rust PromQL parser, validator and evaluator, included in this branch with a versioned C API.
  Operators, functions, vector matching, range selectors and subqueries are checked against the pinned Go engine.
  The expression core also supports native histograms; the Netdata storage adapter currently supplies floats.
- Live Agent and Parent queries using the existing contexts metadata and DBengine pages. Original metadata strings
  are retained by reference. Raw samples stay paged across metrics and time; large computations/results use backing.
- Counter-only page formats at every tier: one 8-byte anchor per page, with retained reset/gap flags. Historical
  counter data remains queryable through approximate cumulative reconstruction. Netdata units and interpolation stay
  in use; this does not recreate original Prometheus scrape samples.
- Six APIs: instant query, range query, series, label names, label values and metric descriptions. Existing host prefixes select one host;
  an unprefixed request includes retained fleet hosts. Local HTTP, ACLK and WebRTC have bounded response delivery.
- Query resource accounting, explicit deadlines and cancellation, conformance checks and real storage/HTTP/peer tests.
  Implicit resource limits remain disabled pending adaptive policy.

Distributed Cloud evaluation, a separate alert-rule lifecycle and Alertmanager are later or excluded work.
See the [API reference](../../src/web/api/promql/README.md) and [design decisions](../promql-conformance/DESIGN.md)
for the detailed contract and qualification boundaries.

## Prerequisites

Use Linux with a C/C++ compiler, Git, CMake 3.16 or newer, Ninja, Rust 1.91 or newer and curl.
Install the development dependencies from the [source build guide](../../packaging/installer/methods/source.md).
The commands below use system Protobuf and `protoc`; both must also be available. Cargo needs network access or
a populated cache for the locked dependencies. This is a focused API build; optional Go collectors and the dashboard
are disabled. Linux proc collection supplies the CPU, load and context-switch examples.

Use a fresh trial directory and fresh DBengine data. New counter pages require this branch's reader; avoid using an
existing production data directory for a trial followed by a downgrade.

## Check out the source

```bash
git clone --single-branch --branch feature/promql-rust-core \
  https://github.com/ktsaou/netdata.git netdata-promql
cd netdata-promql

git submodule update --init --depth 1 src/aclk/aclk-schemas
test -f src/crates/promql-engine/Cargo.toml
```

This branch contains the complete Rust engine, C header, Cargo lockfile and conformance suite as ordinary files.
The library keeps its own Cargo workspace; a separate PromQL repository or repository access is not required.
The existing ACLK schemas still need the submodule command above. Downloading only the Netdata GitHub ZIP omits
those schemas.

## Build and install a private Agent

From the checkout root:

```bash
PROMQL_SOURCE_DIR="$(pwd -P)"
test -f "$PROMQL_SOURCE_DIR/CMakeLists.txt"
test -f "$PROMQL_SOURCE_DIR/src/crates/promql-engine/Cargo.toml"
PROMQL_TRIAL_DIR="$PROMQL_SOURCE_DIR/.tmp/promql-demo"
PROMQL_PREFIX="$PROMQL_TRIAL_DIR/install"

cmake -S "$PROMQL_SOURCE_DIR" -B "$PROMQL_TRIAL_DIR/build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$PROMQL_PREFIX" \
  -DDEFAULT_FEATURE_STATE=Off \
  -DENABLE_PLUGIN_SCRIPTS=Off \
  -DENABLE_DASHBOARD=Off \
  -DENABLE_DBENGINE=On \
  -DENABLE_PROMQL=On \
  -DENABLE_BUNDLED_PROTOBUF=Off

cmake --build "$PROMQL_TRIAL_DIR/build" --parallel 4
cmake --install "$PROMQL_TRIAL_DIR/build"
```

The installed Agent is `"$PROMQL_PREFIX/usr/sbin/netdata"`. The prefix also contains its configuration, stock files,
cache and state directories. This installation does not start a service. If you need bundled Protobuf instead of
system Protobuf, configure with `-DENABLE_BUNDLED_PROTOBUF=On`; that downloads and builds the pinned dependencies.
For a full-featured installation, follow the [normal source installer guide](../../packaging/installer/methods/manual.md)
using this checkout and its initialized submodules.

## Start collection

Write this configuration into the new trial installation:

```bash
cat > "$PROMQL_PREFIX/etc/netdata/netdata.conf" <<'EOF'
[db]
    db = dbengine
    update every = 1
    storage tiers = 1

[web]
    bind to = 127.0.0.1:20001

[plugins]
    proc = yes
    enable running new plugins = no

[health]
    enabled = no

[ml]
    enabled = no
EOF

DO_NOT_TRACK=1 "$PROMQL_PREFIX/usr/sbin/netdata" \
  -D -c "$PROMQL_PREFIX/etc/netdata/netdata.conf"
```

Choose another available port if `20001` is in use, and use the same port in the requests below. Keep this foreground
terminal open. In a second terminal, wait about 15 seconds for several collected samples, then run the queries.
An existing packaged Agent on port `19999` is a different instance and may not contain this API.

## Run queries

All examples select the trial Agent's own host. The API does not require a Prometheus scrape job.

```bash
PROMQL_URL=http://127.0.0.1:20001/host/localhost/api/v1

# Check the expression engine: resultType is scalar and the value is "2".
curl -fsS --get "$PROMQL_URL/query" --data-urlencode 'query=1+1'

# Read a real collected CPU dimension, in percentage units.
curl -fsS --get "$PROMQL_URL/query" \
  --data-urlencode 'query=system_cpu{netdata_dimension_id="user"}'

# Sum non-idle CPU dimensions, keeping each host separate.
curl -fsS --get "$PROMQL_URL/query" \
  --data-urlencode 'query=sum by (netdata_host_id, netdata_host_name) (system_cpu{netdata_dimension_id!="idle"})'

# A range function over a collected gauge.
curl -fsS --get "$PROMQL_URL/query" \
  --data-urlencode 'query=avg_over_time(system_load{netdata_dimension_id="load1"}[5m])'

# Rate of a reconstructed counter: context switches per second.
curl -fsS --get "$PROMQL_URL/query" \
  --data-urlencode 'query=rate(system_ctxt{netdata_dimension_id="switches"}[1m])'
```

Successful responses have `"status":"success"` and query data with `resultType` and `result`. Numerical sample values
are strings; their actual values vary with machine activity. A counter name without `_total` can produce an
informational naming note even when the dimension is genuinely incremental.

`system_cpu` is already a percentage gauge; use it directly rather than applying `rate()`. Names are contexts:
`system_cpu` and `system.cpu` are aliases. Choose one spelling to avoid counting both logical aliases. Exact names
can be selected with `{__name__="system.cpu"}`. Original units remain unchanged.

### Query a time interval

```bash
END="$(date +%s)"
START="$((END - 300))"

curl -fsS --get "$PROMQL_URL/query_range" \
  --data-urlencode 'query=sum by (netdata_host_id, netdata_host_name) (system_cpu{netdata_dimension_id!="idle"})' \
  --data-urlencode "start=$START" \
  --data-urlencode "end=$END" \
  --data-urlencode 'step=5s'
```

The result is a matrix with `values` arrays. A fresh Agent supplies only the history collected since it started;
querying the last five minutes does not manufacture earlier samples. Let it run for five minutes to obtain that
much retained history.

### Discover metrics and labels

```bash
curl -fsS --get "$PROMQL_URL/series" --data-urlencode 'match[]=system_cpu'
curl -fsS --get "$PROMQL_URL/labels" --data-urlencode 'match[]=system_cpu'
curl -fsS --get "$PROMQL_URL/label/netdata_dimension_id/values" \
  --data-urlencode 'match[]=system_load'
curl -fsS --get "$PROMQL_URL/metadata" --data-urlencode 'metric=system_cpu'
```

Labels identify the host, chart, dimension and units. Use them to select and group the desired series.
Metadata descriptions report chart titles, unchanged units and counter/gauge types from contexts, including archived
entries. They do not prove that samples exist in a particular time range.
Removing `/host/localhost` changes these requests to the retained fleet scope on the contacted Agent or Parent.

## Finish and troubleshoot

- Stop the trial with Ctrl-C in its foreground terminal. Its files remain under `.tmp/promql-demo/`.
- A missing library manifest means the Netdata source checkout is incomplete. A CMake dependency error means the named
  build dependency is missing; use the source build guide rather than disabling DBengine or PromQL.
- Connection failure usually means the foreground Agent is not running or the port differs. Inspect its terminal
  and trial log directory under `"$PROMQL_PREFIX/var/log/netdata"`.
- An empty metric result means there are no matching retained series/samples yet. Check collection, selector labels
  and host scope. The scalar `1+1` query distinguishes API availability from metric availability.
- Large queries need writable temporary backing and enough free space. The default is the Agent's cache directory.
  Counter queries crossing more than 16 page-boundary checkpoints also need it, even for a small result.
  An unavailable/full backing location returns an error. Metadata and aggregation state still grow with cardinality.

The [integration checks](README.md) describe the exercised storage, HTTP and response paths. Controlled ACLK tests
are separate from actual HTTP/loopback WebRTC tests; this guide establishes a local installation and query trial.
