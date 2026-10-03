package main

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"os"
	"sort"
	"time"

	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"github.com/prometheus/prometheus/tsdb/chunks"
)

const referenceVersion = "Prometheus v3.15.0 / 5241a27fe3c6983549fccc32f6e65917408c63cd"

type languageFeature struct {
	Name         string             `json:"name"`
	Arguments    []parser.ValueType `json:"arguments,omitempty"`
	Return       parser.ValueType   `json:"return,omitempty"`
	Variadic     int                `json:"variadic,omitempty"`
	Experimental *bool              `json:"experimental,omitempty"`
}

func languageCatalog() map[string]any {
	functions := []languageFeature{}
	for name, f := range parser.Functions {
		functions = append(functions, languageFeature{name, f.ArgTypes, f.ReturnType, f.Variadic, &f.Experimental})
	}
	aggregations, operators, modifiers := []languageFeature{}, []languageFeature{}, []languageFeature{}
	for item, name := range parser.ItemTypeStr {
		switch {
		case item.IsAggregator():
			experimental := item.IsExperimentalAggregator()
			aggregations = append(aggregations, languageFeature{Name: name, Experimental: &experimental})
		case item.IsOperator():
			operators = append(operators, languageFeature{Name: name})
		case item.IsKeyword():
			modifiers = append(modifiers, languageFeature{Name: name})
		}
	}
	for _, fs := range [][]languageFeature{functions, aggregations, operators, modifiers} {
		sort.Slice(fs, func(i, j int) bool { return fs[i].Name < fs[j].Name })
	}
	return map[string]any{"reference": referenceVersion, "functions": functions, "aggregations": aggregations,
		"operators": operators, "modifiers": modifiers}
}

type histogramSample struct{ h *histogram.FloatHistogram }

func (histogramSample) T() int64                        { return 600000 }
func (histogramSample) ST() int64                       { return 0 }
func (histogramSample) F() float64                      { panic("typed histogram sample") }
func (histogramSample) H() *histogram.Histogram         { panic("float histogram sample") }
func (histogramSample) Type() chunkenc.ValueType        { return chunkenc.ValFloatHistogram }
func (s histogramSample) FH() *histogram.FloatHistogram { return s.h.Copy() }
func (s histogramSample) Copy() chunks.Sample           { return histogramSample{s.h.Copy()} }

func histogramFixture() *histogram.FloatHistogram {
	return &histogram.FloatHistogram{CounterResetHint: histogram.GaugeType, Schema: 0, Count: 2, Sum: 1.5,
		PositiveSpans: []histogram.Span{{Offset: 0, Length: 1}}, PositiveBuckets: []float64{2}}
}

type histogramSource struct{}

func (histogramSource) Querier(mint, maxt int64) (storage.Querier, error) {
	return &storage.MockQuerier{SelectMockFunction: func(_ bool, _ *storage.SelectHints, ms ...*labels.Matcher) storage.SeriesSet {
		ls := labels.FromStrings("__name__", "native_hist", "case", "hist")
		selected := []storage.Series{}
		ok := mint <= 600000 && maxt >= 600000
		for _, m := range ms {
			ok = ok && m.Matches(ls.Get(m.Name))
		}
		if ok {
			selected = append(selected, storage.NewListSeries(ls, []chunks.Sample{histogramSample{histogramFixture()}}))
		}
		return &seriesSet{series: selected, index: -1}
	}}, nil
}

func nativeHistogramReference() map[string]any {
	engine := promql.NewEngine(promql.EngineOpts{Logger: slog.New(slog.NewTextHandler(io.Discard, nil)),
		MaxSamples: 1000, Timeout: time.Minute, LookbackDelta: 5 * time.Minute})
	results := []result{}
	for _, fn := range []string{"histogram_count", "histogram_sum", "histogram_avg"} {
		expr := fn + "(native_hist)"
		q, err := engine.NewInstantQuery(context.Background(), histogramSource{}, nil, expr, time.UnixMilli(600000))
		if err != nil {
			panic(err)
		}
		v := q.Exec(context.Background())
		if v.Err != nil {
			panic(v.Err)
		}
		out := result{ID: fn, Query: expr, Kind: 2, Rows: []row{}}
		out.Warnings, out.Infos = v.Warnings.AsStrings(expr, 0, 0)
		for _, sample := range v.Value.(promql.Vector) {
			if sample.H != nil {
				panic("histogram projection must return floats")
			}
			out.Rows = append(out.Rows, singleRow(lab(sample.Metric), sample.T, sample.F))
		}
		q.Close()
		results = append(results, out)
	}
	return map[string]any{"reference": referenceVersion, "fixture": histogramFixture(), "labels": map[string]string{"__name__": "native_hist", "case": "hist"},
		"time_ms": 600000, "results": results}
}

func qualificationCommand(command string) bool {
	var value any
	switch command {
	case "--language-catalog":
		value = languageCatalog()
	case "--native-histogram-probe":
		value = nativeHistogramReference()
	default:
		return false
	}
	if err := json.NewEncoder(os.Stdout).Encode(value); err != nil {
		panic(err)
	}
	return true
}
