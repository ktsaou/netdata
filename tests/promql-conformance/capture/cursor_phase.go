// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only independent source-phase observations over the unmodified pinned engine.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"github.com/prometheus/prometheus/util/annotations"
	"netdata-promql-conformance-oracle/wire"
	"os"
	"time"
)

type phaseFault struct {
	NextTimes   []int64 `json:"next_times,omitempty"`
	SeekTargets []int64 `json:"seek_targets,omitempty"`
	NextEOF     bool    `json:"next_eof,omitempty"`
	SeekEOF     bool    `json:"seek_eof,omitempty"`
	Message     string  `json:"message,omitempty"`
	SetError    string  `json:"set_error,omitempty"`
}
type phaseCase struct {
	Request request               `json:"request"`
	Dataset dataset               `json:"dataset"`
	Faults  map[string]phaseFault `json:"faults"`
}
type phaseEvent struct {
	Event            string                 `json:"event"`
	Name             string                 `json:"name,omitempty"`
	Series           string                 `json:"series,omitempty"`
	Call             int                    `json:"call,omitempty"`
	Kind             string                 `json:"kind,omitempty"`
	Time             *int64                 `json:"time_ms,omitempty"`
	Target           *int64                 `json:"target_ms,omitempty"`
	Error            string                 `json:"error,omitempty"`
	Hints            *storage.SelectHints   `json:"hints,omitempty"`
	Sorted           *bool                  `json:"sorted,omitempty"`
	Histogram        *wire.FloatHistogram   `json:"histogram,omitempty"`
	IntegerHistogram *wire.IntegerHistogram `json:"integer_histogram,omitempty"`
}
type phaseState struct {
	Events []phaseEvent
	Faults map[string]phaseFault
}

func (s *phaseState) add(e phaseEvent) { s.Events = append(s.Events, e) }
func intp(t int64) *int64              { return &t }
func boolp(b bool) *bool               { return &b }
func kind(v chunkenc.ValueType) string {
	switch v {
	case chunkenc.ValNone:
		return "none"
	case chunkenc.ValFloat:
		return "float"
	case chunkenc.ValHistogram:
		return "histogram"
	default:
		return "float_histogram"
	}
}

type phaseSource struct {
	data  dataset
	state *phaseState
}

func (s phaseSource) Querier(mint, maxt int64) (storage.Querier, error) {
	s.state.add(phaseEvent{Event: "querier_open", Time: intp(mint), Target: intp(maxt)})
	q, err := (wire.Source{Dataset: s.data}).Querier(mint, maxt)
	if err != nil {
		return nil, err
	}
	return &phaseQuerier{q, s.state}, nil
}

type phaseQuerier struct {
	storage.Querier
	state *phaseState
}

func (q *phaseQuerier) Select(ctx context.Context, sorted bool, hints *storage.SelectHints, ms ...*labels.Matcher) storage.SeriesSet {
	name := ""
	for _, m := range ms {
		if m.Name == labels.MetricName {
			name = m.Value
		}
	}
	var h *storage.SelectHints
	if hints != nil {
		copy := *hints
		if hints.Grouping != nil {
			copy.Grouping = make([]string, len(hints.Grouping))
			copy.Grouping = append(copy.Grouping[:0], hints.Grouping...)
		}
		h = &copy
	}
	q.state.add(phaseEvent{Event: "select", Name: name, Hints: h, Sorted: boolp(sorted)})
	return &phaseSet{SeriesSet: q.Querier.Select(ctx, sorted, hints, ms...), state: q.state, name: name}
}
func (q *phaseQuerier) Close() error {
	q.state.add(phaseEvent{Event: "querier_close"})
	return q.Querier.Close()
}

type phaseSet struct {
	storage.SeriesSet
	state *phaseState
	name  string
}

func (s *phaseSet) Next() bool {
	ok := s.SeriesSet.Next()
	s.state.add(phaseEvent{Event: "set_next", Name: s.name, Sorted: boolp(ok)})
	return ok
}
func (s *phaseSet) At() storage.Series {
	base := s.SeriesSet.At()
	key := s.name + "/" + base.Labels().Get("z")
	fault, ok := s.state.Faults[key]
	if !ok {
		fault = s.state.Faults[s.name]
	}
	s.state.add(phaseEvent{Event: "set_at", Name: s.name, Series: key})
	return &phaseSeries{base, s.state, key, fault}
}
func (s *phaseSet) Err() error {
	err := s.SeriesSet.Err()
	if msg := s.state.Faults[s.name].SetError; msg != "" {
		err = errors.New(msg)
	}
	text := ""
	if err != nil {
		text = err.Error()
	}
	s.state.add(phaseEvent{Event: "set_error", Name: s.name, Error: text})
	return err
}
func (s *phaseSet) Warnings() annotations.Annotations {
	s.state.add(phaseEvent{Event: "set_warnings", Name: s.name})
	return s.SeriesSet.Warnings()
}

type phaseSeries struct {
	storage.Series
	state *phaseState
	name  string
	fault phaseFault
}

func (s *phaseSeries) Iterator(chunkenc.Iterator) chunkenc.Iterator {
	s.state.add(phaseEvent{Event: "iterator_open", Series: s.name})
	return &phaseIterator{Iterator: s.Series.Iterator(nil), state: s.state, name: s.name, fault: s.fault}
}

type phaseIterator struct {
	chunkenc.Iterator
	state        *phaseState
	name         string
	fault        phaseFault
	failed       string
	nexts, seeks int
}

func member(xs []int64, t int64) bool {
	for _, x := range xs {
		if x == t {
			return true
		}
	}
	return false
}
func (p *phaseIterator) Next() chunkenc.ValueType {
	p.nexts++
	if p.failed != "" {
		p.state.add(phaseEvent{Event: "next", Series: p.name, Call: p.nexts, Kind: "none", Error: p.failed})
		return chunkenc.ValNone
	}
	v := p.Iterator.Next()
	e := phaseEvent{Event: "next", Series: p.name, Call: p.nexts, Kind: kind(v)}
	if v != chunkenc.ValNone {
		e.Time = intp(p.Iterator.AtT())
	}
	if (v == chunkenc.ValNone && p.fault.NextEOF) || (v != chunkenc.ValNone && member(p.fault.NextTimes, p.Iterator.AtT())) {
		p.failed = p.fault.Message
		e.Error = p.failed
		v = chunkenc.ValNone
	}
	p.state.add(e)
	return v
}
func (p *phaseIterator) Seek(t int64) chunkenc.ValueType {
	p.seeks++
	if p.failed != "" {
		p.state.add(phaseEvent{Event: "seek", Series: p.name, Call: p.seeks, Target: intp(t), Kind: "none", Error: p.failed})
		return chunkenc.ValNone
	}
	v := p.Iterator.Seek(t)
	e := phaseEvent{Event: "seek", Series: p.name, Call: p.seeks, Target: intp(t), Kind: kind(v)}
	if v != chunkenc.ValNone {
		e.Time = intp(p.Iterator.AtT())
	}
	if (v == chunkenc.ValNone && p.fault.SeekEOF) || member(p.fault.SeekTargets, t) {
		p.failed = p.fault.Message
		e.Error = p.failed
		v = chunkenc.ValNone
	}
	p.state.add(e)
	return v
}
func (p *phaseIterator) At() (int64, float64) {
	t, v := p.Iterator.At()
	p.state.add(phaseEvent{Event: "at", Series: p.name, Time: intp(t), Kind: "float"})
	return t, v
}
func (p *phaseIterator) AtT() int64 {
	t := p.Iterator.AtT()
	p.state.add(phaseEvent{Event: "at_t", Series: p.name, Time: intp(t)})
	return t
}
func (p *phaseIterator) AtST() int64 {
	t := p.Iterator.AtST()
	p.state.add(phaseEvent{Event: "at_st", Series: p.name, Time: intp(t)})
	return t
}
func (p *phaseIterator) AtHistogram(h *histogram.Histogram) (int64, *histogram.Histogram) {
	t, v := p.Iterator.AtHistogram(h)
	p.state.add(phaseEvent{Event: "at_histogram", Series: p.name, Time: intp(t), IntegerHistogram: wire.EncodeIntegerHistogram(v)})
	return t, v
}
func (p *phaseIterator) AtFloatHistogram(h *histogram.FloatHistogram) (int64, *histogram.FloatHistogram) {
	t, v := p.Iterator.AtFloatHistogram(h)
	p.state.add(phaseEvent{Event: "at_float_histogram", Series: p.name, Time: intp(t), Histogram: wire.EncodeFloatHistogram(v)})
	return t, v
}
func (p *phaseIterator) Err() error {
	err := p.Iterator.Err()
	if p.failed != "" {
		err = errors.New(p.failed)
	}
	text := ""
	if err != nil {
		text = err.Error()
	}
	p.state.add(phaseEvent{Event: "iterator_error", Series: p.name, Error: text})
	return err
}
func phaseAST(e parser.Expr) []map[string]any {
	out := []map[string]any{}
	parser.Inspect(e, func(n parser.Node, path []parser.Node) error {
		if n == nil {
			return nil
		}
		parents := []string{}
		for _, p := range path {
			parents = append(parents, fmt.Sprintf("%T", p))
		}
		item := map[string]any{"type": fmt.Sprintf("%T", n), "parents": parents, "text": n.String()}
		switch x := n.(type) {
		case *parser.VectorSelector:
			item["timestamp_ms"] = x.Timestamp
			item["start_or_end"] = x.StartOrEnd.String()
			item["original_offset_ns"] = int64(x.OriginalOffset)
		case *parser.AggregateExpr:
			item["grouping"] = x.Grouping
			item["without"] = x.Without
		case *parser.SubqueryExpr:
			item["timestamp_ms"] = x.Timestamp
			item["range_ns"] = int64(x.Range)
			item["step_ns"] = int64(x.Step)
		}
		out = append(out, item)
		return nil
	})
	return out
}
func runPhaseProbe(path string) error {
	var cases []phaseCase
	if err := readJSON(path, &cases); err != nil {
		return err
	}
	out := []map[string]any{}
	for _, c := range cases {
		if err := c.Dataset.Validate(); err != nil {
			return err
		}
		text, err := c.Request.Text()
		if err != nil {
			return err
		}
		eng, err := newEngine(c.Request.Profile, c.Request.SubqueryStep)
		if err != nil {
			return err
		}
		state := &phaseState{Events: []phaseEvent{}, Faults: c.Faults}
		source := phaseSource{c.Dataset, state}
		lookback := c.Request.Lookback
		if lookback == 0 {
			lookback = 300000
		}
		opts := promql.NewPrometheusQueryOpts(false, time.Duration(lookback)*time.Millisecond)
		po, err := parserOptions(c.Request.Profile)
		if err != nil {
			return err
		}
		ast, err := parser.NewParser(po).ParseExpr(text)
		if err != nil {
			return err
		}
		rawAST := phaseAST(ast)
		var q promql.Query
		if c.Request.Mode == "range" {
			q, err = eng.NewRangeQuery(context.Background(), source, opts, text, time.UnixMilli(c.Request.Start), time.UnixMilli(c.Request.End), time.Duration(c.Request.Step)*time.Millisecond)
		} else {
			q, err = eng.NewInstantQuery(context.Background(), source, opts, text, time.UnixMilli(c.Request.Time))
		}
		item := map[string]any{"request": c.Request, "raw_ast": rawAST}
		if err != nil {
			item["construction_error"] = encodeError("construction", err)
			item["events"] = state.Events
			out = append(out, item)
			eng.Close()
			continue
		}
		stmt := q.Statement().(*parser.EvalStmt)
		mint, maxt := promql.FindMinMaxTime(stmt)
		item["prepared_ast"] = phaseAST(stmt.Expr)
		item["header"] = map[string]any{"start_ms": stmt.Start.UnixMilli(), "end_ms": stmt.End.UnixMilli(), "interval_ns": int64(stmt.Interval), "lookback_ns": int64(stmt.LookbackDelta), "mint": mint, "maxt": maxt}
		item["result"] = encodeResult(c.Request.ID, text, q.Exec(context.Background()))
		item["executed_ast"] = phaseAST(stmt.Expr)
		q.Close()
		eng.Close()
		item["events"] = state.Events
		out = append(out, item)
	}
	return json.NewEncoder(os.Stdout).Encode(envelope("cases", out))
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: cursor-oracle inputs.json")
		os.Exit(2)
	}
	if err := runPhaseProbe(os.Args[1]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
