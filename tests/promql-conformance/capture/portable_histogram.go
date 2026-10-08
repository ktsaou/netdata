// SPDX-License-Identifier: GPL-3.0-or-later
// Observe unchanged pinned Prometheus arithmetic; see the conformance guide.
package main

import (
	"encoding/json"
	"fmt"
	"github.com/prometheus/prometheus/model/histogram"
	"math"
	"netdata-promql-conformance-oracle/wire"
	"os"
	"strconv"
)

type Case struct {
	ID, Mode            string
	Lower, Upper, Value string
	Linear, UpperTrim   bool
	Histogram           *wire.FloatHistogram
}

func f(s string) float64 {
	u, e := strconv.ParseUint(s, 16, 64)
	if e != nil {
		panic(e)
	}
	return math.Float64frombits(u)
}
func bits(v float64) string { return fmt.Sprintf("%016x", math.Float64bits(v)) }
func spanSnapshot(s []histogram.Span) []histogram.Span {
	if s == nil {
		return nil
	}
	out := make([]histogram.Span, len(s))
	copy(out, s)
	return out
}
func floatsSnapshot(s []float64) []string {
	if s == nil {
		return nil
	}
	out := make([]string, len(s))
	for i, v := range s {
		out[i] = bits(v)
	}
	return out
}
func state(h *histogram.FloatHistogram) any {
	if h == nil {
		return nil
	}
	return map[string]any{"zero_count": bits(h.ZeroCount), "count": bits(h.Count), "sum": bits(h.Sum), "zero_threshold": bits(h.ZeroThreshold), "schema": h.Schema, "hint": h.CounterResetHint, "positive": floatsSnapshot(h.PositiveBuckets), "negative": floatsSnapshot(h.NegativeBuckets), "positive_spans": spanSnapshot(h.PositiveSpans), "negative_spans": spanSnapshot(h.NegativeSpans), "custom": floatsSnapshot(h.CustomValues)}
}
func run(c Case) (out map[string]any) {
	out = map[string]any{"id": c.ID, "request": c}
	defer func() {
		if x := recover(); x != nil {
			out["panic"] = fmt.Sprint(x)
		}
	}()
	if c.Mode == "fraction" {
		b := histogram.Bucket[float64]{Lower: f(c.Lower), Upper: f(c.Upper), Count: 1}
		out["result"] = bits(b.FractionBelow(f(c.Value), c.Linear))
		return
	}
	h := c.Histogram.Decode()
	out["before"] = state(h)
	h.TrimBuckets(f(c.Value), c.UpperTrim)
	out["after"] = state(h)
	return
}
func main() {
	raw, e := os.ReadFile(os.Args[1])
	if e != nil {
		panic(e)
	}
	var in []Case
	if e = json.Unmarshal(raw, &in); e != nil {
		panic(e)
	}
	enc := json.NewEncoder(os.Stdout)
	for _, c := range in {
		if e = enc.Encode(run(c)); e != nil {
			panic(e)
		}
	}
}
