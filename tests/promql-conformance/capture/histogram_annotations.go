// SPDX-License-Identifier: GPL-3.0-or-later
// Capture original annotation constructors and merge behavior before the Rust port.
package main

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"reflect"
	"runtime"
	"sort"
	"strconv"

	"github.com/prometheus/prometheus/promql/parser/posrange"
	"github.com/prometheus/prometheus/util/annotations"
)

type event struct {
	Op         string `json:"op"`
	Kind       string `json:"kind"`
	Metric     string `json:"metric"`
	Label      string `json:"label"`
	Raw        string `json:"raw"`
	Severity   string `json:"severity"`
	Start      int64  `json:"start"`
	End        int64  `json:"end"`
	NoPosition bool   `json:"no_position"`
	Time       int64  `json:"time_ms"`
	Min        string `json:"min"`
	Max        string `json:"max"`
	Diff       string `json:"diff"`
	Q          string `json:"q"`
}
type request struct {
	ID     string  `json:"id"`
	Query  string  `json:"query"`
	Events []event `json:"events"`
}
type span struct {
	Start int64 `json:"start"`
	End   int64 `json:"end"`
}
type note struct {
	Raw         string `json:"raw"`
	Message     string `json:"message"`
	Severity    string `json:"severity"`
	Span        *span  `json:"span"`
	Occurrences int64  `json:"occurrences"`
}
type result struct {
	ID    string `json:"id"`
	Notes []note `json:"notes"`
}

func decode(text string) string {
	raw, err := hex.DecodeString(text)
	if err != nil {
		panic(err)
	}
	return string(raw)
}
func float(text string) float64 {
	bits, err := strconv.ParseUint(text, 16, 64)
	if err != nil || len(text) != 16 {
		panic("invalid binary64 bits")
	}
	return math.Float64frombits(bits)
}

type opaque struct {
	Raw      string
	Severity string
	Position *posrange.PositionRange
	Query    string
}

func (e *opaque) Error() string {
	if e.Query == "" || e.Position == nil {
		return e.Raw
	}
	return fmt.Sprintf("%s (%s)", e.Raw, e.Position.StartPosInput(e.Query, 0))
}
func (e *opaque) Unwrap() error {
	if e.Severity == "info" {
		return annotations.PromQLInfo
	}
	if e.Severity == "warning" {
		return annotations.PromQLWarning
	}
	panic("invalid opaque severity")
}
func (e *opaque) SetQuery(query string) { e.Query = query }
func (e *opaque) Merge(_ error) error   { return e }

func makeNote(e event) error {
	pos := posrange.PositionRange{Start: posrange.Pos(e.Start), End: posrange.Pos(e.End)}
	metric := decode(e.Metric)
	switch e.Kind {
	case "monotonic":
		return annotations.NewHistogramQuantileForcedMonotonicityInfo(metric, pos, e.Time, float(e.Min), float(e.Max), float(e.Diff))
	case "invalid_quantile":
		return annotations.NewInvalidQuantileWarning(float(e.Q), pos)
	case "bad_bucket":
		return annotations.NewBadBucketLabelWarning(metric, decode(e.Label), pos)
	case "mixed_classic_native":
		return annotations.NewMixedClassicNativeHistogramsWarning(metric, pos)
	case "quantile_nan_result":
		return annotations.NewNativeHistogramQuantileNaNResultInfo(metric, pos)
	case "quantile_nan_skew":
		return annotations.NewNativeHistogramQuantileNaNSkewInfo(metric, pos)
	case "fraction_nans":
		return annotations.NewNativeHistogramFractionNaNsInfo(metric, pos)
	case "opaque":
		var position *posrange.PositionRange
		if !e.NoPosition {
			position = &pos
		}
		return &opaque{Raw: decode(e.Raw), Severity: e.Severity, Position: position}
	default:
		panic("unknown annotation kind")
	}
}
func capture(req request) result {
	var parent, child annotations.Annotations
	for _, event := range req.Events {
		switch event.Op {
		case "add":
			parent.Add(makeNote(event))
		case "child_add":
			child.Add(makeNote(event))
		case "merge":
			parent.Merge(child)
			child = nil
		default:
			panic("unknown merge operation")
		}
	}
	query := decode(req.Query)
	keys := make([]string, 0, len(parent))
	for key := range parent {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	// Invoke the actual final formatting boundary after all raw-key merges.
	parent.AsStrings(query, 0, 0)
	out := result{ID: req.ID, Notes: make([]note, 0, len(keys))}
	for _, key := range keys {
		err := parent[key]
		severity := "warning"
		if errors.Is(err, annotations.PromQLInfo) {
			severity = "info"
		}
		count := int64(1)
		var position *span
		if e, ok := err.(*opaque); ok {
			if e.Position != nil {
				position = &span{int64(e.Position.Start), int64(e.Position.End)}
			}
		} else {
			value := reflect.ValueOf(err).Elem()
			field := value.FieldByName("PositionRange")
			if !field.IsValid() {
				panic("original annotation position owner changed")
			}
			position = &span{field.FieldByName("Start").Int(), field.FieldByName("End").Int()}
			if value.Type().Name() == "histogramQuantileForcedMonotonicityErr" {
				field = value.FieldByName("count")
				if !field.IsValid() {
					panic("original monotonic count owner changed")
				}
				count = field.Int() + 1
			}
		}
		out.Notes = append(out.Notes, note{hex.EncodeToString([]byte(key)), hex.EncodeToString([]byte(err.Error())), severity, position, count})
	}
	return out
}
func main() {
	var requests []request
	decoder := json.NewDecoder(os.Stdin)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]result, 0, len(requests))
	for _, request := range requests {
		results = append(results, capture(request))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"go_version": runtime.Version(), "architecture": runtime.GOARCH, "results": results}); err != nil {
		panic(err)
	}
}
