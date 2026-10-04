// SPDX-License-Identifier: GPL-3.0-or-later
package promqltest

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io/fs"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"sort"
	"testing"
	"time"
	"unicode"

	codec "github.com/prometheus/prometheus/internal/netdatacodec"
	wire "github.com/prometheus/prometheus/internal/netdatawire"
	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/tsdb"
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"github.com/prometheus/prometheus/util/teststorage"
	"github.com/stretchr/testify/require"
)

type captureCommand struct {
	ID           string `json:"id"`
	File         string `json:"file"`
	Line         int    `json:"line"`
	Query        []byte `json:"query"`
	Mode         string `json:"mode"`
	Failure      bool   `json:"expected_failure"`
	Ordered      bool   `json:"ordered"`
	Warn         bool   `json:"warn"`
	Info         bool   `json:"info"`
	ExcludeRange bool   `json:"exclude_generated_range"`
	Assertions   any    `json:"assertions"`
}

type captureOptions struct {
	Nil      bool  `json:"nil"`
	PerStep  bool  `json:"per_step"`
	Lookback int64 `json:"lookback_ns"`
}

type captureContext struct {
	Start    int64 `json:"start_ms"`
	End      int64 `json:"end_ms"`
	Interval int64 `json:"interval_ns"`
	Lookback int64 `json:"lookback_ns"`
}

type captureEntry struct {
	Command        captureCommand  `json:"source_command"`
	Ordinal        int             `json:"variant_ordinal"`
	Dataset        string          `json:"dataset_id"`
	Request        wire.Request    `json:"request"`
	Options        captureOptions  `json:"query_options"`
	Context        *captureContext `json:"effective_context,omitempty"`
	AssertionScope string          `json:"original_assertion_scope"`
	Result         wire.Result     `json:"result"`
	ReplayExact    bool            `json:"fixture_replay_exact"`
}

type captureEngine struct {
	*promql.Engine
	tb        testing.TB
	command   *evalCmd
	meta      captureCommand
	ordinal   int
	dataset   *wire.Dataset
	datasetID string
	directory string
	writer    *json.Encoder
	entries   int
	datasets  map[string]bool
	err       error
}

// Snapshot the committed storage, including generated NHCBs and decoded STs.
func captureDataset(ctx context.Context, queryable storage.Queryable) (out wire.Dataset, err error) {
	out = wire.Dataset{Schema: wire.SchemaVersion, Series: []wire.Series{}}
	querier, err := queryable.Querier(math.MinInt64, math.MaxInt64)
	if err != nil {
		return out, err
	}
	defer func() {
		if closeErr := querier.Close(); err == nil {
			err = closeErr
		}
	}()
	set := querier.Select(ctx, false, nil, labels.MustNewMatcher(labels.MatchRegexp, "__name__", ".*"))
	for set.Next() {
		series := set.At()
		row := wire.Series{Labels: wire.EncodeLabels(series.Labels()), Points: []wire.Point{}}
		it := series.Iterator(nil)
		for kind := it.Next(); kind != chunkenc.ValNone; kind = it.Next() {
			var point wire.Point
			switch kind {
			case chunkenc.ValFloat:
				t, f := it.At()
				point = wire.FloatPoint(t, f)
			case chunkenc.ValHistogram:
				t, h := it.AtHistogram(nil)
				point = wire.Point{Time: t, Kind: "histogram", Histogram: wire.EncodeIntegerHistogram(h)}
			case chunkenc.ValFloatHistogram:
				t, h := it.AtFloatHistogram(nil)
				point = wire.Point{Time: t, Kind: "float_histogram", FloatHistogram: wire.EncodeFloatHistogram(h)}
			default:
				return out, fmt.Errorf("unknown stored sample kind %v", kind)
			}
			point.StartTime = it.AtST()
			row.Points = append(row.Points, point)
		}
		if err := it.Err(); err != nil {
			return out, err
		}
		out.Series = append(out.Series, row)
	}
	if err := set.Err(); err != nil {
		return out, err
	}
	if warnings := set.Warnings(); len(warnings) != 0 {
		return out, fmt.Errorf("snapshot storage annotations: %v", warnings)
	}
	return out, out.Validate()
}

func (c *captureEngine) input(ctx context.Context, queryable storage.Queryable) error {
	if c.dataset != nil {
		return nil
	}
	dataset, err := captureDataset(ctx, queryable)
	if err != nil {
		return err
	}
	raw, err := json.Marshal(dataset)
	if err != nil {
		return err
	}
	id := fmt.Sprintf("%x", sha256.Sum256(raw))
	path := filepath.Join(c.directory, "datasets", id+".json")
	if !c.datasets[id] {
		if err := captureWriteNew(path, raw); err != nil {
			return err
		}
		c.datasets[id] = true
	}
	c.dataset, c.datasetID = &dataset, id
	return nil
}

func captureWriteNew(path string, raw []byte) error {
	f, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	_, err = f.Write(raw)
	if closeErr := f.Close(); err == nil {
		err = closeErr
	}
	return err
}

func (c *captureEngine) entry(ctx context.Context, queryable storage.Queryable, opts promql.QueryOpts, request wire.Request) (captureEntry, error) {
	if c.err != nil {
		return captureEntry{}, c.err
	}
	if err := c.input(ctx, queryable); err != nil {
		c.err = err
		return captureEntry{}, err
	}
	request.ID = fmt.Sprintf("%s:variant:%04d", c.meta.ID, c.ordinal)
	request.Profile, request.Lookback, request.SubqueryStep = "upstream-tests", 300000, 60000
	entry := captureEntry{Command: c.meta, Ordinal: c.ordinal, Dataset: c.datasetID, Request: request,
		Options: captureOptions{Nil: opts == nil}}
	c.ordinal++
	if opts != nil {
		entry.Options.PerStep = opts.EnablePerStepStats()
		entry.Options.Lookback = int64(opts.LookbackDelta())
	}
	switch {
	case c.command.isRange:
		entry.AssertionScope = "source-range-all-points"
	case request.Mode == "instant":
		entry.AssertionScope = "source-or-generated-instant-result"
	case c.command.isOrdered():
		entry.AssertionScope = "generated-range-execution-only"
	default:
		entry.AssertionScope = "generated-range-middle-step-and-label-order"
	}
	return entry, nil
}

func (c *captureEngine) NewInstantQuery(ctx context.Context, queryable storage.Queryable, opts promql.QueryOpts, query string, at time.Time) (promql.Query, error) {
	entry, err := c.entry(ctx, queryable, opts, wire.Request{QueryBytes: []byte(query), Mode: "instant", Time: at.UnixMilli()})
	if err != nil {
		c.tb.Errorf("capture input: %v", err)
		return nil, err
	}
	q, err := c.Engine.NewInstantQuery(ctx, queryable, opts, query, at)
	return c.wrap(ctx, entry, opts, q, err)
}

func (c *captureEngine) NewRangeQuery(ctx context.Context, queryable storage.Queryable, opts promql.QueryOpts, query string, start, end time.Time, step time.Duration) (promql.Query, error) {
	entry, err := c.entry(ctx, queryable, opts, wire.Request{QueryBytes: []byte(query), Mode: "range", Start: start.UnixMilli(), End: end.UnixMilli(), Step: step.Milliseconds()})
	if err != nil {
		c.tb.Errorf("capture input: %v", err)
		return nil, err
	}
	q, err := c.Engine.NewRangeQuery(ctx, queryable, opts, query, start, end, step)
	return c.wrap(ctx, entry, opts, q, err)
}

func (c *captureEngine) wrap(ctx context.Context, entry captureEntry, opts promql.QueryOpts, q promql.Query, err error) (promql.Query, error) {
	if err != nil {
		entry.Result = wire.Result{ID: entry.Request.ID, Kind: "error", Rows: []wire.Series{},
			Warnings: [][]byte{}, Infos: [][]byte{}, Error: codec.Error("construction", err)}
		c.record(ctx, &entry, opts)
		return nil, err
	}
	stmt := q.Statement().(*parser.EvalStmt)
	entry.Context = &captureContext{stmt.Start.UnixMilli(), stmt.End.UnixMilli(), int64(stmt.Interval), int64(stmt.LookbackDelta)}
	return &captureQuery{Query: q, owner: c, entry: entry, opts: opts}, nil
}

type captureQuery struct {
	promql.Query
	owner *captureEngine
	entry captureEntry
	opts  promql.QueryOpts
}

func (q *captureQuery) Exec(ctx context.Context) *promql.Result {
	result := q.Query.Exec(ctx)
	query, _ := q.entry.Request.Text()
	q.entry.Result = codec.Result(q.entry.Request.ID, query, result)
	q.owner.record(ctx, &q.entry, q.opts)
	return result
}

func (c *captureEngine) record(ctx context.Context, entry *captureEntry, opts promql.QueryOpts) {
	request := entry.Request
	query, _ := request.Text()
	var replay promql.Query
	var err error
	input := wire.Source{Dataset: *c.dataset}
	if request.Mode == "instant" {
		replay, err = c.Engine.NewInstantQuery(ctx, input, opts, query, time.UnixMilli(request.Time))
	} else {
		replay, err = c.Engine.NewRangeQuery(ctx, input, opts, query, time.UnixMilli(request.Start), time.UnixMilli(request.End), time.Duration(request.Step)*time.Millisecond)
	}
	var expected wire.Result
	if err != nil {
		expected = wire.Result{ID: request.ID, Kind: "error", Rows: []wire.Series{}, Warnings: [][]byte{}, Infos: [][]byte{}, Error: codec.Error("construction", err)}
	} else {
		expected = codec.Result(request.ID, query, replay.Exec(ctx))
		replay.Close()
	}
	entry.ReplayExact = reflect.DeepEqual(captureCanonical(entry.Result, entry.Command.Ordered), captureCanonical(expected, entry.Command.Ordered))
	if !entry.ReplayExact {
		raw, _ := json.Marshal(map[string]any{"entry": entry, "replay": expected})
		path := filepath.Join(c.directory, fmt.Sprintf("replay-difference-%x.json", sha256.Sum256([]byte(request.ID))))
		if err := captureWriteNew(path, raw); err != nil {
			c.tb.Errorf("writing replay difference: %v", err)
		}
		c.tb.Errorf("fixture replay differs for %s; evidence: %s", request.ID, path)
	}
	if err := c.writer.Encode(entry); err != nil {
		c.err = err
		c.tb.Errorf("writing capture: %v", err)
	}
	c.entries++
}

// Empty histogram arrays have identical meaning; preserve every numeric bit and reset hint.
func captureCanonical(result wire.Result, ordered bool) wire.Result {
	result.Rows = append([]wire.Series{}, result.Rows...)
	for i := range result.Rows {
		row := &result.Rows[i]
		row.Points = append([]wire.Point{}, row.Points...)
		for j := range row.Points {
			point := &row.Points[j]
			if point.FloatHistogram == nil {
				continue
			}
			h := *point.FloatHistogram
			if len(h.PositiveSpans) == 0 {
				h.PositiveSpans = nil
			}
			if len(h.NegativeSpans) == 0 {
				h.NegativeSpans = nil
			}
			if len(h.PositiveBuckets) == 0 {
				h.PositiveBuckets = nil
			}
			if len(h.NegativeBuckets) == 0 {
				h.NegativeBuckets = nil
			}
			if len(h.CustomValues) == 0 {
				h.CustomValues = nil
			}
			point.FloatHistogram = &h
		}
	}
	if !ordered {
		sort.Slice(result.Rows, func(i, j int) bool {
			a, _ := result.Rows[i].Labels.Decode()
			b, _ := result.Rows[j].Labels.Decode()
			return labels.Compare(a, b) < 0
		})
	}
	return result
}

func captureAssertions(cmd *evalCmd) any {
	out := map[string]any{"failure_message": []byte(cmd.expectedFailMessage)}
	if cmd.expectedFailRegexp != nil {
		out["failure_regexp"] = []byte(cmd.expectedFailRegexp.String())
	}
	ordered := []map[string]any{}
	for kind := Ordered; kind <= NoInfo; kind++ {
		for _, assertion := range cmd.expectedCmds[kind] {
			ordered = append(ordered, map[string]any{"kind": int(kind), "type": assertion.Type(), "value": []byte(assertion.String())})
		}
	}
	out["commands"] = ordered
	return out
}

func TestNetdataCaptureEvaluation(t *testing.T) {
	directory := os.Getenv("NETDATA_PROMQL_CAPTURE_EVALUATION")
	if directory == "" {
		t.Skip("capture directory not configured")
	}
	require.NoError(t, os.Mkdir(filepath.Join(directory, "datasets"), 0o700))
	file, err := os.OpenFile(filepath.Join(directory, "evaluation.jsonl"), os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	require.NoError(t, err)
	defer file.Close()
	c := &captureEngine{Engine: NewTestEngine(t, false, 5*time.Minute, 10000),
		directory: directory, writer: json.NewEncoder(file), datasets: map[string]bool{}}
	files, err := fs.Glob(testsFs, "*/*.test")
	require.NoError(t, err)
	require.Len(t, files, 21)
	commands := []captureCommand{}
	for _, name := range files {
		t.Run(name, func(t *testing.T) {
			content, err := fs.ReadFile(testsFs, name)
			require.NoError(t, err)
			instance, err := newTest(t, string(content), false, func(tb testing.TB) storage.Storage {
				return teststorage.New(tb, func(options *tsdb.Options) {
					options.EnableSTStorage = true
					options.FloatChunkEncoding = chunkenc.EncXOR2
					options.EnableHistogramSTEncoding = true
				})
			})
			defer func() {
				if instance == nil {
					return
				}
				if instance.storage != nil {
					require.NoError(t, instance.storage.Close())
				}
				if instance.cancelCtx != nil {
					instance.cancelCtx()
				}
			}()
			require.NoError(t, err)
			c.tb = t
			c.dataset = nil
			for _, command := range instance.cmds {
				if eval, ok := command.(*evalCmd); ok {
					mode := "instant"
					if eval.isRange {
						mode = "range"
					}
					c.meta = captureCommand{ID: fmt.Sprintf("%s:%d", name, eval.line), File: name, Line: eval.line,
						Query: []byte(eval.expr), Mode: mode, Failure: eval.isFail(), Ordered: eval.isOrdered(),
						Warn: eval.warn, Info: eval.info, ExcludeRange: eval.excludeFromRangeQuery, Assertions: captureAssertions(eval)}
					commands = append(commands, c.meta)
					c.command, c.ordinal = eval, 0
				} else {
					c.dataset = nil
				}
				require.NoError(t, instance.exec(command, c))
			}
		})
	}
	require.NoError(t, c.err)
	require.Equal(t, 2243, len(commands), "all computed source commands")
	require.NoError(t, file.Close())
	if t.Failed() {
		return
	}
	metadata := map[string]any{"schema_version": 1, "commit": wire.ReferenceCommit,
		"go_version": runtime.Version(), "go_os": runtime.GOOS, "go_arch": runtime.GOARCH,
		"unicode_version": unicode.Version, "original_assertions_passed": true, "fixture_replays_exact": true,
		"fixture_replay_normalization": "Unordered result rows and nil/empty histogram arrays only; all numerical bits exact.",
		"engine_options": map[string]any{"max_samples": 10000, "timeout_ns": int64(100 * time.Second),
			"lookback_ns": int64(5 * time.Minute), "subquery_step_ms": 60000, "enable_at": true,
			"enable_negative_offset": true, "enable_delayed_name_removal": true, "use_start_timestamps": true,
			"per_step_stats": false, "parser": TestParserOpts},
		"commands": commands, "entries": c.entries, "datasets": len(c.datasets)}
	raw, err := json.MarshalIndent(metadata, "", "  ")
	require.NoError(t, err)
	require.NoError(t, captureWriteNew(filepath.Join(directory, "evaluation-metadata.json"), append(bytes.TrimSpace(raw), '\n')))
}
