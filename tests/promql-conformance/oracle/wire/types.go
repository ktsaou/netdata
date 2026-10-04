// SPDX-License-Identifier: GPL-3.0-or-later
package wire

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

const SchemaVersion = 1
const ReferenceCommit = "5241a27fe3c6983549fccc32f6e65917408c63cd"
const ReferenceModule = "github.com/prometheus/prometheus@v0.315.0"

// Binary64 transport keeps stale NaNs and signed zero distinct from normal NaNs and zero.
type Binary64 uint64

func Number(f float64) Binary64   { return Binary64(math.Float64bits(f)) }
func (f Binary64) Value() float64 { return math.Float64frombits(uint64(f)) }
func (f Binary64) MarshalJSON() ([]byte, error) {
	return json.Marshal(fmt.Sprintf("%016x", uint64(f)))
}
func (f *Binary64) UnmarshalJSON(b []byte) error {
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
	*f = Binary64(v)
	return nil
}

// Go strings can contain arbitrary bytes; JSON text alone is not a lossless transport.
type Label struct {
	Name  []byte `json:"name"`
	Value []byte `json:"value"`
}
type Labelset []Label

func EncodeLabels(ls labels.Labels) Labelset {
	out := Labelset{}
	ls.Range(func(l labels.Label) { out = append(out, Label{[]byte(l.Name), []byte(l.Value)}) })
	return out
}
func (ls Labelset) Decode() (labels.Labels, error) {
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

type FloatHistogram struct {
	ResetHint       histogram.CounterResetHint `json:"reset_hint"`
	Schema          int32                      `json:"schema"`
	ZeroThreshold   Binary64                   `json:"zero_threshold"`
	ZeroCount       Binary64                   `json:"zero_count"`
	Count           Binary64                   `json:"count"`
	Sum             Binary64                   `json:"sum"`
	PositiveSpans   []histogram.Span           `json:"positive_spans"`
	NegativeSpans   []histogram.Span           `json:"negative_spans"`
	PositiveBuckets []Binary64                 `json:"positive_buckets"`
	NegativeBuckets []Binary64                 `json:"negative_buckets"`
	CustomValues    []Binary64                 `json:"custom_values"`
}
type IntegerHistogram struct {
	ResetHint       histogram.CounterResetHint `json:"reset_hint"`
	Schema          int32                      `json:"schema"`
	ZeroThreshold   Binary64                   `json:"zero_threshold"`
	ZeroCount       uint64                     `json:"zero_count"`
	Count           uint64                     `json:"count"`
	Sum             Binary64                   `json:"sum"`
	PositiveSpans   []histogram.Span           `json:"positive_spans"`
	NegativeSpans   []histogram.Span           `json:"negative_spans"`
	PositiveBuckets []int64                    `json:"positive_buckets"`
	NegativeBuckets []int64                    `json:"negative_buckets"`
	CustomValues    []Binary64                 `json:"custom_values"`
}

func EncodeNumbers(fs []float64) []Binary64 {
	if fs == nil {
		return nil
	}
	out := make([]Binary64, len(fs))
	for i, f := range fs {
		out[i] = Number(f)
	}
	return out
}
func DecodeNumbers(fs []Binary64) []float64 {
	if fs == nil {
		return nil
	}
	out := make([]float64, len(fs))
	for i, f := range fs {
		out[i] = f.Value()
	}
	return out
}
func EncodeFloatHistogram(h *histogram.FloatHistogram) *FloatHistogram {
	return &FloatHistogram{h.CounterResetHint, h.Schema, Number(h.ZeroThreshold), Number(h.ZeroCount),
		Number(h.Count), Number(h.Sum), append([]histogram.Span(nil), h.PositiveSpans...),
		append([]histogram.Span(nil), h.NegativeSpans...), EncodeNumbers(h.PositiveBuckets),
		EncodeNumbers(h.NegativeBuckets), EncodeNumbers(h.CustomValues)}
}
func (h *FloatHistogram) Decode() *histogram.FloatHistogram {
	return &histogram.FloatHistogram{CounterResetHint: h.ResetHint, Schema: h.Schema,
		ZeroThreshold: h.ZeroThreshold.Value(), ZeroCount: h.ZeroCount.Value(), Count: h.Count.Value(),
		Sum: h.Sum.Value(), PositiveSpans: append([]histogram.Span(nil), h.PositiveSpans...),
		NegativeSpans: append([]histogram.Span(nil), h.NegativeSpans...), PositiveBuckets: DecodeNumbers(h.PositiveBuckets),
		NegativeBuckets: DecodeNumbers(h.NegativeBuckets), CustomValues: DecodeNumbers(h.CustomValues)}
}
func EncodeIntegerHistogram(h *histogram.Histogram) *IntegerHistogram {
	return &IntegerHistogram{h.CounterResetHint, h.Schema, Number(h.ZeroThreshold), h.ZeroCount, h.Count,
		Number(h.Sum), append([]histogram.Span(nil), h.PositiveSpans...), append([]histogram.Span(nil), h.NegativeSpans...),
		append([]int64(nil), h.PositiveBuckets...), append([]int64(nil), h.NegativeBuckets...), EncodeNumbers(h.CustomValues)}
}
func (h *IntegerHistogram) Decode() *histogram.Histogram {
	return &histogram.Histogram{CounterResetHint: h.ResetHint, Schema: h.Schema, ZeroThreshold: h.ZeroThreshold.Value(),
		ZeroCount: h.ZeroCount, Count: h.Count, Sum: h.Sum.Value(), PositiveSpans: append([]histogram.Span(nil), h.PositiveSpans...),
		NegativeSpans: append([]histogram.Span(nil), h.NegativeSpans...), PositiveBuckets: append([]int64(nil), h.PositiveBuckets...),
		NegativeBuckets: append([]int64(nil), h.NegativeBuckets...), CustomValues: DecodeNumbers(h.CustomValues)}
}

type Point struct {
	Time           int64             `json:"time_ms"`
	StartTime      int64             `json:"start_time_ms,omitempty"`
	Kind           string            `json:"kind"`
	Float          *Binary64         `json:"float,omitempty"`
	Histogram      *IntegerHistogram `json:"histogram,omitempty"`
	FloatHistogram *FloatHistogram   `json:"float_histogram,omitempty"`
	String         []byte            `json:"string,omitempty"`
}

func FloatPoint(t int64, f float64) Point {
	bits := Number(f)
	return Point{Time: t, Kind: "float", Float: &bits}
}

type Series struct {
	Labels Labelset `json:"labels"`
	Points []Point  `json:"points"`
}
type Dataset struct {
	Schema int      `json:"schema_version"`
	Series []Series `json:"series"`
}

func (d Dataset) Validate() error {
	if d.Schema != SchemaVersion {
		return fmt.Errorf("unsupported dataset schema %d", d.Schema)
	}
	seen := map[string]bool{}
	for _, s := range d.Series {
		ls, err := s.Labels.Decode()
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

type Request struct {
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

func (r Request) Text() (string, error) {
	if r.QueryBytes != nil {
		if r.Query != "" {
			return "", fmt.Errorf("query and query_bytes are mutually exclusive")
		}
		return string(r.QueryBytes), nil
	}
	return r.Query, nil
}

type ParseFailure struct {
	Start      int    `json:"start"`
	End        int    `json:"end"`
	LineOffset int    `json:"line_offset"`
	Message    []byte `json:"message"`
}
type Failure struct {
	Phase       string         `json:"phase"`
	Type        string         `json:"type"`
	Message     []byte         `json:"message"`
	ParseErrors []ParseFailure `json:"parse_errors,omitempty"`
}
type Result struct {
	ID       string   `json:"id"`
	Kind     string   `json:"kind"`
	Rows     []Series `json:"rows"`
	Warnings [][]byte `json:"warnings"`
	Infos    [][]byte `json:"infos"`
	Error    *Failure `json:"error,omitempty"`
}

func ByteMessages(ss []string) [][]byte {
	out := make([][]byte, 0, len(ss))
	for _, s := range ss {
		out = append(out, []byte(s))
	}
	sort.Slice(out, func(i, j int) bool { return bytes.Compare(out[i], out[j]) < 0 })
	return out
}
