// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"sort"
	"strconv"

	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
)

const schemaVersion = 1
const referenceCommit = "5241a27fe3c6983549fccc32f6e65917408c63cd"
const referenceModule = "github.com/prometheus/prometheus@v0.315.0"

// Binary64 transport keeps stale NaNs and signed zero distinct from normal NaNs and zero.
type binary64 uint64

func number(f float64) binary64   { return binary64(math.Float64bits(f)) }
func (f binary64) value() float64 { return math.Float64frombits(uint64(f)) }
func (f binary64) MarshalJSON() ([]byte, error) {
	return json.Marshal(fmt.Sprintf("%016x", uint64(f)))
}
func (f *binary64) UnmarshalJSON(b []byte) error {
	var s string
	if err := json.Unmarshal(b, &s); err != nil {
		return err
	}
	if len(s) != 16 {
		return fmt.Errorf("binary64 needs 16 hex digits")
	}
	v, err := strconv.ParseUint(s, 16, 64)
	if err != nil {
		return err
	}
	*f = binary64(v)
	return nil
}

// Go strings can contain arbitrary bytes; JSON text alone is not a lossless transport.
type label struct {
	Name  []byte `json:"name"`
	Value []byte `json:"value"`
}
type labelset []label

func encodeLabels(ls labels.Labels) labelset {
	out := labelset{}
	ls.Range(func(l labels.Label) { out = append(out, label{[]byte(l.Name), []byte(l.Value)}) })
	return out
}
func (ls labelset) decode() (labels.Labels, error) {
	strings := make([]string, 0, len(ls)*2)
	seen := map[string]bool{}
	for _, l := range ls {
		name := string(l.Name)
		if name == "" || seen[name] {
			return labels.EmptyLabels(), fmt.Errorf("empty or duplicate label name")
		}
		seen[name] = true
		strings = append(strings, name, string(l.Value))
	}
	return labels.FromStrings(strings...), nil
}

type floatHistogram struct {
	ResetHint       histogram.CounterResetHint `json:"reset_hint"`
	Schema          int32                      `json:"schema"`
	ZeroThreshold   binary64                   `json:"zero_threshold"`
	ZeroCount       binary64                   `json:"zero_count"`
	Count           binary64                   `json:"count"`
	Sum             binary64                   `json:"sum"`
	PositiveSpans   []histogram.Span           `json:"positive_spans"`
	NegativeSpans   []histogram.Span           `json:"negative_spans"`
	PositiveBuckets []binary64                 `json:"positive_buckets"`
	NegativeBuckets []binary64                 `json:"negative_buckets"`
	CustomValues    []binary64                 `json:"custom_values"`
}
type integerHistogram struct {
	ResetHint       histogram.CounterResetHint `json:"reset_hint"`
	Schema          int32                      `json:"schema"`
	ZeroThreshold   binary64                   `json:"zero_threshold"`
	ZeroCount       uint64                     `json:"zero_count"`
	Count           uint64                     `json:"count"`
	Sum             binary64                   `json:"sum"`
	PositiveSpans   []histogram.Span           `json:"positive_spans"`
	NegativeSpans   []histogram.Span           `json:"negative_spans"`
	PositiveBuckets []int64                    `json:"positive_buckets"`
	NegativeBuckets []int64                    `json:"negative_buckets"`
	CustomValues    []binary64                 `json:"custom_values"`
}

func encodeNumbers(fs []float64) []binary64 {
	if fs == nil {
		return nil
	}
	out := make([]binary64, len(fs))
	for i, f := range fs {
		out[i] = number(f)
	}
	return out
}
func decodeNumbers(fs []binary64) []float64 {
	if fs == nil {
		return nil
	}
	out := make([]float64, len(fs))
	for i, f := range fs {
		out[i] = f.value()
	}
	return out
}
func encodeFloatHistogram(h *histogram.FloatHistogram) *floatHistogram {
	return &floatHistogram{h.CounterResetHint, h.Schema, number(h.ZeroThreshold), number(h.ZeroCount),
		number(h.Count), number(h.Sum), append([]histogram.Span(nil), h.PositiveSpans...),
		append([]histogram.Span(nil), h.NegativeSpans...), encodeNumbers(h.PositiveBuckets),
		encodeNumbers(h.NegativeBuckets), encodeNumbers(h.CustomValues)}
}
func (h *floatHistogram) decode() *histogram.FloatHistogram {
	return &histogram.FloatHistogram{CounterResetHint: h.ResetHint, Schema: h.Schema,
		ZeroThreshold: h.ZeroThreshold.value(), ZeroCount: h.ZeroCount.value(), Count: h.Count.value(),
		Sum: h.Sum.value(), PositiveSpans: append([]histogram.Span(nil), h.PositiveSpans...),
		NegativeSpans: append([]histogram.Span(nil), h.NegativeSpans...), PositiveBuckets: decodeNumbers(h.PositiveBuckets),
		NegativeBuckets: decodeNumbers(h.NegativeBuckets), CustomValues: decodeNumbers(h.CustomValues)}
}
func encodeIntegerHistogram(h *histogram.Histogram) *integerHistogram {
	return &integerHistogram{h.CounterResetHint, h.Schema, number(h.ZeroThreshold), h.ZeroCount, h.Count,
		number(h.Sum), append([]histogram.Span(nil), h.PositiveSpans...), append([]histogram.Span(nil), h.NegativeSpans...),
		append([]int64(nil), h.PositiveBuckets...), append([]int64(nil), h.NegativeBuckets...), encodeNumbers(h.CustomValues)}
}
func (h *integerHistogram) decode() *histogram.Histogram {
	return &histogram.Histogram{CounterResetHint: h.ResetHint, Schema: h.Schema, ZeroThreshold: h.ZeroThreshold.value(),
		ZeroCount: h.ZeroCount, Count: h.Count, Sum: h.Sum.value(), PositiveSpans: append([]histogram.Span(nil), h.PositiveSpans...),
		NegativeSpans: append([]histogram.Span(nil), h.NegativeSpans...), PositiveBuckets: append([]int64(nil), h.PositiveBuckets...),
		NegativeBuckets: append([]int64(nil), h.NegativeBuckets...), CustomValues: decodeNumbers(h.CustomValues)}
}

type point struct {
	Time           int64             `json:"time_ms"`
	StartTime      int64             `json:"start_time_ms,omitempty"`
	Kind           string            `json:"kind"`
	Float          *binary64         `json:"float,omitempty"`
	Histogram      *integerHistogram `json:"histogram,omitempty"`
	FloatHistogram *floatHistogram   `json:"float_histogram,omitempty"`
	String         []byte            `json:"string,omitempty"`
}

func floatPoint(t int64, f float64) point {
	bits := number(f)
	return point{Time: t, Kind: "float", Float: &bits}
}

type series struct {
	Labels labelset `json:"labels"`
	Points []point  `json:"points"`
}
type dataset struct {
	Schema int      `json:"schema_version"`
	Series []series `json:"series"`
}

func (d dataset) validate() error {
	if d.Schema != schemaVersion {
		return fmt.Errorf("unsupported dataset schema %d", d.Schema)
	}
	seen := map[string]bool{}
	for _, s := range d.Series {
		ls, err := s.Labels.decode()
		if err != nil {
			return err
		}
		key := string(ls.Bytes(nil))
		if seen[key] {
			return fmt.Errorf("duplicate input labelset")
		}
		seen[key] = true
		for i, p := range s.Points {
			if i > 0 && s.Points[i-1].Time >= p.Time {
				return fmt.Errorf("input timestamps must increase strictly")
			}
			payloads := 0
			if p.Float != nil {
				payloads++
			}
			if p.Histogram != nil {
				payloads++
			}
			if p.FloatHistogram != nil {
				payloads++
			}
			if payloads != 1 || len(p.String) != 0 {
				return fmt.Errorf("sample needs exactly one numeric payload")
			}
			switch p.Kind {
			case "float":
				if p.Float == nil {
					return fmt.Errorf("float payload missing")
				}
			case "histogram":
				if p.Histogram == nil {
					return fmt.Errorf("histogram payload missing")
				}
			case "float_histogram":
				if p.FloatHistogram == nil {
					return fmt.Errorf("float histogram payload missing")
				}
			default:
				return fmt.Errorf("invalid input sample kind %q", p.Kind)
			}
		}
	}
	return nil
}

type request struct {
	ID           string `json:"id"`
	Query        string `json:"query,omitempty"`
	QueryBytes   []byte `json:"query_bytes,omitempty"`
	Profile      string `json:"profile"`
	Mode         string `json:"mode"`
	Time         int64  `json:"time_ms"`
	Start        int64  `json:"start_ms"`
	End          int64  `json:"end_ms"`
	Step         int64  `json:"step_ms"`
	Lookback     int64  `json:"lookback_ms"`
	SubqueryStep int64  `json:"subquery_step_ms"`
}

func (r request) text() (string, error) {
	if r.QueryBytes != nil {
		if r.Query != "" {
			return "", fmt.Errorf("query and query_bytes are mutually exclusive")
		}
		return string(r.QueryBytes), nil
	}
	return r.Query, nil
}

type parseFailure struct {
	Start      int    `json:"start"`
	End        int    `json:"end"`
	LineOffset int    `json:"line_offset"`
	Message    []byte `json:"message"`
}
type failure struct {
	Phase       string         `json:"phase"`
	Type        string         `json:"type"`
	Message     []byte         `json:"message"`
	ParseErrors []parseFailure `json:"parse_errors,omitempty"`
}
type result struct {
	ID       string   `json:"id"`
	Kind     string   `json:"kind"`
	Rows     []series `json:"rows"`
	Warnings [][]byte `json:"warnings"`
	Infos    [][]byte `json:"infos"`
	Error    *failure `json:"error,omitempty"`
}

func byteMessages(ss []string) [][]byte {
	out := make([][]byte, 0, len(ss))
	for _, s := range ss {
		out = append(out, []byte(s))
	}
	sort.Slice(out, func(i, j int) bool { return bytes.Compare(out[i], out[j]) < 0 })
	return out
}
