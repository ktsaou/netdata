// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"math"
	"os"
	"runtime"
	"sort"
	"time"
	"unicode"

	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"github.com/prometheus/prometheus/util/almost"
)

func parserOptions(profile string) (parser.Options, error) {
	switch profile {
	case "server-default":
		return parser.Options{ExperimentalDurationExpr: true}, nil
	case "upstream-tests":
		return parser.Options{ExperimentalDurationExpr: true, EnableExperimentalFunctions: true, EnableExtendedRangeSelectors: true, EnableBinopFillModifiers: true}, nil
	default:
		return parser.Options{}, fmt.Errorf("unknown profile %q", profile)
	}
}
func newEngine(profile string, subqueryStep int64) (*promql.Engine, error) {
	options, err := parserOptions(profile)
	if err != nil {
		return nil, err
	}
	if subqueryStep == 0 {
		subqueryStep = 60000
	}
	if subqueryStep < 1 || subqueryStep > math.MaxInt64/int64(time.Millisecond) {
		return nil, fmt.Errorf("subquery step must be positive")
	}
	return promql.NewEngine(promql.EngineOpts{Logger: slog.New(slog.NewTextHandler(io.Discard, nil)),
		MaxSamples: 10000000, Timeout: time.Minute, LookbackDelta: 5 * time.Minute,
		EnableAtModifier: true, EnableNegativeOffset: true, EnableDelayedNameRemoval: profile == "upstream-tests",
		UseStartTimestamps: profile == "upstream-tests", NoStepSubqueryIntervalFn: func(int64) int64 { return subqueryStep },
		Parser: parser.NewParser(options)}), nil
}
func encodeError(phase string, err error) *failure {
	out := &failure{Phase: phase, Type: fmt.Sprintf("%T", err), Message: []byte(err.Error())}
	var parsed parser.ParseErrors
	if errors.As(err, &parsed) {
		for _, e := range parsed {
			out.ParseErrors = append(out.ParseErrors, parseFailure{int(e.PositionRange.Start), int(e.PositionRange.End), e.LineOffset, []byte(e.Err.Error())})
		}
	}
	return out
}
func encodeResult(id, query string, v *promql.Result) result {
	warnings, infos := v.Warnings.AsStrings(query, 0, 0)
	out := result{ID: id, Rows: []series{}, Warnings: byteMessages(warnings), Infos: byteMessages(infos)}
	if v.Err != nil {
		out.Kind = "error"
		out.Error = encodeError("execution", v.Err)
		return out
	}
	switch x := v.Value.(type) {
	case promql.Scalar:
		out.Kind = "scalar"
		out.Rows = append(out.Rows, series{labelset{}, []point{floatPoint(x.T, x.V)}})
	case promql.String:
		out.Kind = "string"
		out.Rows = append(out.Rows, series{labelset{}, []point{{Time: x.T, Kind: "string", String: []byte(x.V)}}})
	case promql.Vector:
		out.Kind = "vector"
		for _, s := range x {
			p := floatPoint(s.T, s.F)
			if s.H != nil {
				p = point{Time: s.T, Kind: "float_histogram", FloatHistogram: encodeFloatHistogram(s.H)}
			}
			out.Rows = append(out.Rows, series{encodeLabels(s.Metric), []point{p}})
		}
	case promql.Matrix:
		out.Kind = "matrix"
		for _, s := range x {
			points := []point{}
			for _, p := range s.Floats {
				points = append(points, floatPoint(p.T, p.F))
			}
			for _, p := range s.Histograms {
				points = append(points, point{Time: p.T, Kind: "float_histogram", FloatHistogram: encodeFloatHistogram(p.H)})
			}
			sort.Slice(points, func(i, j int) bool { return points[i].Time < points[j].Time })
			out.Rows = append(out.Rows, series{encodeLabels(s.Metric), points})
		}
	default:
		panic(fmt.Sprintf("unhandled upstream value %T", x))
	}
	return out
}
func evaluate(d dataset, r request) result {
	out := result{ID: r.ID, Kind: "error", Rows: []series{}, Warnings: [][]byte{}, Infos: [][]byte{}}
	query, err := r.text()
	if err != nil {
		out.Error = encodeError("request", err)
		return out
	}
	engine, err := newEngine(r.Profile, r.SubqueryStep)
	if err != nil {
		out.Error = encodeError("request", err)
		return out
	}
	lookback := r.Lookback
	if lookback == 0 {
		lookback = 300000
	}
	if lookback < 1 || lookback > math.MaxInt64/int64(time.Millisecond) {
		out.Error = encodeError("request", fmt.Errorf("lookback must be positive"))
		return out
	}
	opts := promql.NewPrometheusQueryOpts(false, time.Duration(lookback)*time.Millisecond)
	var q promql.Query
	switch r.Mode {
	case "instant":
		q, err = engine.NewInstantQuery(context.Background(), source{d}, opts, query, time.UnixMilli(r.Time))
	case "range":
		if r.Step < 1 || r.Step > math.MaxInt64/int64(time.Millisecond) || r.End < r.Start {
			out.Error = encodeError("request", fmt.Errorf("range needs positive step and ordered endpoints"))
			return out
		}
		q, err = engine.NewRangeQuery(context.Background(), source{d}, opts, query, time.UnixMilli(r.Start), time.UnixMilli(r.End), time.Duration(r.Step)*time.Millisecond)
	default:
		out.Error = encodeError("request", fmt.Errorf("unknown mode %q", r.Mode))
		return out
	}
	if err != nil {
		out.Error = encodeError("construction", err)
		return out
	}
	defer q.Close()
	return encodeResult(r.ID, query, q.Exec(context.Background()))
}

type analysis struct {
	ID           string           `json:"id"`
	Type         parser.ValueType `json:"type,omitempty"`
	Functions    []string         `json:"functions"`
	Aggregations []string         `json:"aggregations"`
	Operators    []string         `json:"operators"`
	Error        *failure         `json:"error,omitempty"`
}

func analyze(r request) analysis {
	out := analysis{ID: r.ID, Functions: []string{}, Aggregations: []string{}, Operators: []string{}}
	query, err := r.text()
	if err != nil {
		out.Error = encodeError("request", err)
		return out
	}
	opts, err := parserOptions(r.Profile)
	if err != nil {
		out.Error = encodeError("request", err)
		return out
	}
	expr, err := parser.NewParser(opts).ParseExpr(query)
	if err != nil {
		out.Error = encodeError("construction", err)
		return out
	}
	out.Type = expr.Type()
	fs, as, ops := map[string]bool{}, map[string]bool{}, map[string]bool{}
	parser.Inspect(expr, func(node parser.Node, _ []parser.Node) error {
		switch n := node.(type) {
		case *parser.Call:
			fs[n.Func.Name] = true
		case *parser.AggregateExpr:
			as[n.Op.String()] = true
		case *parser.BinaryExpr:
			ops[n.Op.String()] = true
		}
		return nil
	})
	for f := range fs {
		out.Functions = append(out.Functions, f)
	}
	for a := range as {
		out.Aggregations = append(out.Aggregations, a)
	}
	for op := range ops {
		out.Operators = append(out.Operators, op)
	}
	sort.Strings(out.Functions)
	sort.Strings(out.Aggregations)
	sort.Strings(out.Operators)
	return out
}
func readJSON(path string, v any) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	d := json.NewDecoder(f)
	d.DisallowUnknownFields()
	if err = d.Decode(v); err != nil {
		return err
	}
	var extra any
	if err = d.Decode(&extra); err != io.EOF {
		return fmt.Errorf("trailing JSON data")
	}
	return nil
}

func envelope(field string, value any) map[string]any {
	return map[string]any{"schema_version": schemaVersion, "reference": referenceModule, "commit": referenceCommit,
		"go_version": runtime.Version(), "go_os": runtime.GOOS, "go_arch": runtime.GOARCH,
		"unicode_version": unicode.Version, field: value}
}

func catalog() map[string]any {
	functions := []map[string]any{}
	for name, f := range parser.Functions {
		functions = append(functions, map[string]any{"name": name, "arguments": f.ArgTypes,
			"return": f.ReturnType, "variadic": f.Variadic, "experimental": f.Experimental})
	}
	sort.Slice(functions, func(i, j int) bool { return functions[i]["name"].(string) < functions[j]["name"].(string) })
	aggregations, operators := []map[string]any{}, []string{}
	for item, name := range parser.ItemTypeStr {
		if item.IsAggregator() {
			aggregations = append(aggregations, map[string]any{"name": name, "experimental": item.IsExperimentalAggregator()})
		}
		if item.IsOperator() {
			operators = append(operators, name)
		}
	}
	sort.Slice(aggregations, func(i, j int) bool { return aggregations[i]["name"].(string) < aggregations[j]["name"].(string) })
	sort.Strings(operators)
	profiles := map[string]any{}
	for _, name := range []string{"server-default", "upstream-tests"} {
		options, _ := parserOptions(name)
		profiles[name] = map[string]any{"parser": options, "at_modifier": true, "negative_offset": true,
			"delayed_name_removal": name == "upstream-tests", "start_timestamps": name == "upstream-tests"}
	}
	out := envelope("functions", functions)
	out["aggregations"], out["operators"], out["profiles"] = aggregations, operators, profiles
	return out
}

func run(args []string) error {
	if len(args) == 1 && args[0] == "--catalog" {
		return json.NewEncoder(os.Stdout).Encode(catalog())
	}
	if len(args) == 2 && args[0] == "--numbers" {
		var pairs [][2]binary64
		if err := readJSON(args[1], &pairs); err != nil {
			return err
		}
		out := make([]bool, len(pairs))
		for i, pair := range pairs {
			out[i] = almost.Equal(pair[0].value(), pair[1].value(), 1e-6)
		}
		return json.NewEncoder(os.Stdout).Encode(out)
	}
	if len(args) == 2 && args[0] == "--compact-histograms" {
		var inputs []floatHistogram
		if err := readJSON(args[1], &inputs); err != nil {
			return err
		}
		out := make([]*floatHistogram, len(inputs))
		for i, h := range inputs {
			out[i] = encodeFloatHistogram(h.decode().Compact(0))
		}
		return json.NewEncoder(os.Stdout).Encode(out)
	}
	if len(args) == 2 && args[0] == "--analyze" {
		var requests []request
		if err := readJSON(args[1], &requests); err != nil {
			return err
		}
		if err := validateRequestIDs(requests); err != nil {
			return err
		}
		out := []analysis{}
		for _, r := range requests {
			out = append(out, analyze(r))
		}
		return json.NewEncoder(os.Stdout).Encode(envelope("analyses", out))
	}
	if len(args) != 2 {
		return fmt.Errorf("usage: oracle dataset.json requests.json | --analyze requests.json")
	}
	var data dataset
	var requests []request
	if err := readJSON(args[0], &data); err != nil {
		return err
	}
	if err := data.validate(); err != nil {
		return err
	}
	if err := readJSON(args[1], &requests); err != nil {
		return err
	}
	if err := validateRequestIDs(requests); err != nil {
		return err
	}
	out := []result{}
	for _, r := range requests {
		out = append(out, evaluate(data, r))
	}
	return json.NewEncoder(os.Stdout).Encode(envelope("results", out))
}

func validateRequestIDs(requests []request) error {
	seen := map[string]bool{}
	for _, r := range requests {
		if r.ID == "" || seen[r.ID] {
			return fmt.Errorf("empty or duplicate request id")
		}
		seen[r.ID] = true
	}
	return nil
}
func main() {
	if err := run(os.Args[1:]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
