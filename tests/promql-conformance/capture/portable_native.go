// SPDX-License-Identifier: GPL-3.0-or-later
// Observe unchanged pinned Prometheus arithmetic; see the conformance guide.
package main

import (
	"encoding/json"
	"fmt"
	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"math"
	"os"
	"strconv"
)

type Native struct{ ID, Shape, Op, A, B string }
type Reduction struct{ ID, Op, A, B string }
type Function struct{ ID, Function, A, B, Scope string }
type Width struct {
	ID, Mode, A, Op, Text, Origin string
	Exponent                      int64
	Offset                        int32
}
type Input struct {
	Native    []Native
	Reduction []Reduction
	Functions []Function
	Widths    []Width
}

func f(s string) float64 {
	u, e := strconv.ParseUint(s, 16, 64)
	if e != nil {
		panic(e)
	}
	return math.Float64frombits(u)
}
func bits(v float64) string { return fmt.Sprintf("%016x", math.Float64bits(v)) }
func h(v float64) *histogram.FloatHistogram {
	return &histogram.FloatHistogram{Schema: 0, ZeroThreshold: .001, ZeroCount: v, Count: v, Sum: v, PositiveSpans: []histogram.Span{{Offset: 0, Length: 2}}, PositiveBuckets: []float64{v, v}, NegativeSpans: []histogram.Span{{Offset: 0, Length: 1}}, NegativeBuckets: []float64{v}}
}
func spanSnapshot(s []histogram.Span) []histogram.Span {
	if s == nil {
		return nil
	}
	out := make([]histogram.Span, len(s))
	copy(out, s)
	return out
}
func state(h *histogram.FloatHistogram) any {
	if h == nil {
		return nil
	}
	p, n, c := []string{}, []string{}, []string{}
	for _, v := range h.PositiveBuckets {
		p = append(p, bits(v))
	}
	for _, v := range h.NegativeBuckets {
		n = append(n, bits(v))
	}
	for _, v := range h.CustomValues {
		c = append(c, bits(v))
	}
	return map[string]any{"zero_count": bits(h.ZeroCount), "count": bits(h.Count), "sum": bits(h.Sum), "zero_threshold": bits(h.ZeroThreshold), "schema": h.Schema, "hint": h.CounterResetHint, "positive": p, "negative": n, "positive_spans": spanSnapshot(h.PositiveSpans), "negative_spans": spanSnapshot(h.NegativeSpans), "custom": c}
}
func native(r Native) (out map[string]any) {
	out = map[string]any{"id": r.ID, "input": r}
	defer func() {
		if v := recover(); v != nil {
			out["panic"] = fmt.Sprint(v)
		}
	}()
	a, b := h(f(r.A)), h(f(r.B))
	switch r.Shape {
	case "new":
		a.PositiveSpans[0].Offset = 2
		a.NegativeSpans[0].Offset = 2
	case "reduce":
		a.Schema = 1
		a.PositiveSpans[0].Offset = 1
	case "threshold":
		b.ZeroThreshold = 1.1
	case "custom-match", "custom-reconcile":
		for _, x := range []*histogram.FloatHistogram{a, b} {
			x.Schema = histogram.CustomBucketsSchema
			x.ZeroThreshold = 0
			x.ZeroCount = 0
			x.NegativeSpans = nil
			x.NegativeBuckets = nil
			x.PositiveSpans = []histogram.Span{{Offset: 0, Length: 3}}
			x.PositiveBuckets = []float64{x.Count, x.Count, x.Count}
			x.CustomValues = []float64{.5, 1}
		}
		if r.Shape == "custom-reconcile" {
			b.CustomValues = []float64{1, 2}
		}
	}
	out["before"] = state(a)
	out["other"] = state(b)
	var e error
	var reset, reconcile bool
	switch r.Op {
	case "add":
		_, reset, reconcile, e = a.Add(b)
	case "sub":
		_, reset, reconcile, e = a.Sub(b)
	case "kahan-add-final":
		var c *histogram.FloatHistogram
		c, reset, reconcile, e = a.KahanAdd(b, nil)
		out["before_final"] = state(a)
		out["compensation"] = state(c)
		if e == nil && c != nil {
			_, _, _, e = a.Add(c)
		}
	}
	out["raw"] = state(a)
	out["reset"] = reset
	out["reconciled"] = reconcile
	if e != nil {
		out["error"] = e.Error()
	}
	out["other_after"] = state(b)
	out["compact"] = state(a.Copy().Compact(0))
	return
}
func reduction(r Reduction) (out map[string]any) {
	out = map[string]any{"id": r.ID, "input": r}
	defer func() {
		if v := recover(); v != nil {
			out["panic"] = fmt.Sprint(v)
		}
	}()
	a := h(1)
	a.ZeroCount = 0
	a.PositiveSpans = []histogram.Span{{Offset: 1, Length: 2}}
	a.PositiveBuckets = []float64{f(r.A), f(r.B)}
	a.NegativeSpans = []histogram.Span{{Offset: 1, Length: 2}}
	a.NegativeBuckets = []float64{f(r.B), f(r.A)}
	out["before"] = state(a)
	if r.Op == "copy" {
		out["raw"] = state(a.CopyToSchema(-1))
	} else {
		if e := a.ReduceResolution(-1); e != nil {
			out["error"] = e.Error()
		}
		out["raw"] = state(a)
	}
	out["input_after"] = state(a)
	return
}
func call(r Function) (out map[string]any) {
	out = map[string]any{"id": r.ID, "input": r}
	defer func() {
		if v := recover(); v != nil {
			out["panic"] = fmt.Sprint(v)
		}
	}()
	s := r.Function + "(m[10s])"
	if r.Function == "double_exponential_smoothing" {
		s = r.Function + "(m[10s],0.4,0.6)"
	}
	p, e := parser.NewParser(parser.Options{EnableExperimentalFunctions: true}).ParseExpr(s)
	if e != nil {
		panic(e)
	}
	args := p.(*parser.Call).Args
	points := []promql.FPoint{{T: -3000, F: 1}, {T: -2000, F: f(r.A)}, {T: -1000, F: f(r.B)}, {T: 0, F: 3}}
	if r.Scope == "endpoints" {
		points = []promql.FPoint{{T: -3000, F: f(r.A)}, {T: -2000, F: 1}, {T: -1000, F: 3}, {T: 0, F: f(r.B)}}
	}
	if r.Scope == "overflow" {
		points = []promql.FPoint{{T: -3000, F: math.MaxFloat64}, {T: -2000, F: math.MaxFloat64}, {T: -1000, F: f(r.A)}, {T: 0, F: f(r.B)}}
	}
	if r.Scope == "nan-factor" {
		points = []promql.FPoint{{T: -3000, F: 1}, {T: -2000, F: 2}, {T: -1000, F: 3}, {T: 0, F: 4}}
	}
	matrix := promql.Matrix{{Floats: points}}
	var vectors []promql.Vector
	if r.Function == "double_exponential_smoothing" {
		vectors = []promql.Vector{{{F: .4}}, {{F: .6}}}
	}
	enh := &promql.EvalNodeHelper{Ts: 0}
	if r.Scope == "nan-factor" {
		vectors = []promql.Vector{{{F: f(r.A)}}, {{F: f(r.B)}}}
	}
	if r.Scope == "start-reset" {
		enh.StartTimestamps = &promql.StartTimestamps{Floats: []int64{-4000, -2500, -1500, -500}}
	}
	if r.Scope == "start-first" {
		enh.StartTimestamps = &promql.StartTimestamps{Floats: []int64{-3500, 0, 0, 0}}
	}
	v, ann := promql.FunctionCalls[r.Function](vectors, matrix, args, enh)
	o := []any{}
	for _, s := range v {
		o = append(o, map[string]any{"float": bits(s.F), "histogram": state(s.H), "drop_name": s.DropName})
	}
	out["output"] = o
	out["annotations"] = len(ann)
	return
}
func width(r Width) (out map[string]any) {
	out = map[string]any{"id": r.ID, "input": r, "int_width": strconv.IntSize}
	defer func() {
		if v := recover(); v != nil {
			out["panic"] = fmt.Sprint(v)
		}
	}()
	switch r.Mode {
	case "ldexp":
		out["int_exponent"] = int(r.Exponent)
		out["result"] = bits(math.Ldexp(f(r.A), int(r.Exponent)))
	case "compact":
		a := h(1)
		a.ZeroCount = 0
		a.PositiveSpans = []histogram.Span{{Offset: 0, Length: 1}, {Offset: 4, Length: 1}}
		a.PositiveBuckets = []float64{1, 1}
		a.NegativeSpans = nil
		a.NegativeBuckets = nil
		out["before"] = state(a)
		out["int_max_empty"] = int(r.Exponent)
		out["raw"] = state(a.Compact(int(r.Exponent)))
	case "cumulative":
		a, b := h(1), h(1)
		for _, x := range []*histogram.FloatHistogram{a, b} {
			x.Schema = histogram.CustomBucketsSchema
			x.ZeroThreshold = 0
			x.ZeroCount = 0
			x.Count = 2
			x.Sum = 2
			x.NegativeSpans = nil
			x.NegativeBuckets = nil
			x.PositiveSpans = []histogram.Span{{Offset: r.Offset, Length: 1}, {Offset: r.Offset, Length: 1}}
			x.PositiveBuckets = []float64{1, 1}
		}
		a.CustomValues = []float64{.5, 1}
		b.CustomValues = []float64{1, 2}
		out["before"] = state(a)
		out["other"] = state(b)
		var e error
		if r.Op == "add" {
			_, _, _, e = a.Add(b)
		} else {
			_, _, _, e = a.Sub(b)
		}
		if e != nil {
			out["error"] = e.Error()
		}
		out["raw"] = state(a)
	case "decimal":
		v, e := strconv.ParseFloat(r.Text, 64)
		out["result"] = bits(v)
		if e != nil {
			out["error"] = e.Error()
		}
	}
	return
}
func main() {
	raw, e := os.ReadFile(os.Args[1])
	if e != nil {
		panic(e)
	}
	var in Input
	if e = json.Unmarshal(raw, &in); e != nil {
		panic(e)
	}
	enc := json.NewEncoder(os.Stdout)
	for _, r := range in.Native {
		if e := enc.Encode(native(r)); e != nil {
			panic(e)
		}
	}
	for _, r := range in.Reduction {
		if e := enc.Encode(reduction(r)); e != nil {
			panic(e)
		}
	}
	for _, r := range in.Functions {
		if e := enc.Encode(call(r)); e != nil {
			panic(e)
		}
	}
	for _, r := range in.Widths {
		if e := enc.Encode(width(r)); e != nil {
			panic(e)
		}
	}
}
