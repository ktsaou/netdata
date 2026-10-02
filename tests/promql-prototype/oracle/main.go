package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"math"
	"os"
	"sort"
	"strconv"
	"time"

	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"github.com/prometheus/prometheus/util/annotations"
)

type inputSeries struct {
	Labels map[string]string    `json:"labels"`
	Points [][2]json.RawMessage `json:"points"`
}
type dataset struct {
	Series []inputSeries `json:"series"`
}
type request struct {
	ID       string `json:"id"`
	Query    string `json:"query"`
	Time     int64  `json:"time_ms"`
	Lookback int64  `json:"lookback_ms"`
	Start    int64  `json:"start_ms"`
	End      int64  `json:"end_ms"`
	Step     int64  `json:"step_ms"`
}
type row struct {
	Labels map[string]string `json:"labels"`
	Points [][2]any          `json:"points"`
}
type result struct {
	ID       string   `json:"id"`
	Query    string   `json:"query"`
	Kind     int      `json:"kind"`
	Warnings []string `json:"warnings,omitempty"`
	Infos    []string `json:"infos,omitempty"`
	Rows     []row    `json:"rows"`
	Error    string   `json:"error,omitempty"`
}
type seriesSet struct {
	series []storage.Series
	index  int
}

func (s *seriesSet) Next() bool                        { s.index++; return s.index < len(s.series) }
func (s *seriesSet) At() storage.Series                { return s.series[s.index] }
func (s *seriesSet) Err() error                        { return nil }
func (s *seriesSet) Warnings() annotations.Annotations { return nil }

type source struct{ data dataset }
type floatSeries struct {
	ls labels.Labels
	ts []int64
	vs []float64
}

func (s floatSeries) Labels() labels.Labels { return s.ls }
func (s floatSeries) Iterator(chunkenc.Iterator) chunkenc.Iterator {
	return &floatIterator{ts: s.ts, vs: s.vs, i: -1}
}

type floatIterator struct {
	ts []int64
	vs []float64
	i  int
}

func (it *floatIterator) Next() chunkenc.ValueType {
	it.i++
	if it.i >= len(it.ts) {
		return chunkenc.ValNone
	}
	return chunkenc.ValFloat
}
func (it *floatIterator) Seek(t int64) chunkenc.ValueType {
	if it.i >= len(it.ts) {
		return chunkenc.ValNone
	}
	if it.i >= 0 && it.i < len(it.ts) && it.ts[it.i] >= t {
		return chunkenc.ValFloat
	}
	it.i = sort.Search(len(it.ts), func(i int) bool { return it.ts[i] >= t })
	if it.i == len(it.ts) {
		return chunkenc.ValNone
	}
	return chunkenc.ValFloat
}
func (it *floatIterator) At() (int64, float64) { return it.ts[it.i], it.vs[it.i] }
func (it *floatIterator) AtT() int64           { return it.ts[it.i] }
func (it *floatIterator) AtST() int64          { return 0 }
func (it *floatIterator) AtHistogram(*histogram.Histogram) (int64, *histogram.Histogram) {
	panic("float iterator")
}
func (it *floatIterator) AtFloatHistogram(*histogram.FloatHistogram) (int64, *histogram.FloatHistogram) {
	panic("float iterator")
}
func (it *floatIterator) Err() error { return nil }
func (q source) Querier(mint, maxt int64) (storage.Querier, error) {
	return &storage.MockQuerier{SelectMockFunction: func(sorted bool, hints *storage.SelectHints, ms ...*labels.Matcher) storage.SeriesSet {
		selected := []storage.Series{}
		for _, s := range q.data.Series {
			ok := true
			for _, m := range ms {
				if !m.Matches(s.Labels[m.Name]) {
					ok = false
					break
				}
			}
			if !ok {
				continue
			}
			ts := []int64{}
			vs := []float64{}
			ls := []string{}
			for k, v := range s.Labels {
				ls = append(ls, k, v)
			}
			for _, p := range s.Points {
				var t int64
				if err := json.Unmarshal(p[0], &t); err != nil {
					panic(err)
				}
				if t < mint || t > maxt {
					continue
				}
				ts = append(ts, t)
				vs = append(vs, float(p[1]))
			}
			selected = append(selected, floatSeries{labels.FromStrings(ls...), ts, vs})
		}
		if sorted {
			sort.Slice(selected, func(i, j int) bool { return labels.Compare(selected[i].Labels(), selected[j].Labels()) < 0 })
		}
		return &seriesSet{series: selected, index: -1}
	}}, nil
}
func float(b json.RawMessage) float64 {
	var f float64
	if json.Unmarshal(b, &f) == nil {
		return f
	}
	var s string
	if err := json.Unmarshal(b, &s); err != nil {
		panic(err)
	}
	switch s {
	case "NaN":
		return math.NaN()
	case "+Inf":
		return math.Inf(1)
	case "-Inf":
		return math.Inf(-1)
	}
	panic("invalid numeric sample")
}
func val(f float64) string { return strconv.FormatFloat(f, 'g', 17, 64) }
func lab(ls labels.Labels) map[string]string {
	out := map[string]string{}
	ls.Range(func(l labels.Label) { out[l.Name] = l.Value })
	return out
}
func read(path string, v any) {
	b, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	if err = json.Unmarshal(b, v); err != nil {
		panic(err)
	}
}
func main() {
	if len(os.Args) != 3 {
		panic("usage: oracle dataset.json requests.json")
	}
	var d dataset
	var rs []request
	read(os.Args[1], &d)
	read(os.Args[2], &rs)
	engine := promql.NewEngine(promql.EngineOpts{Logger: slog.New(slog.NewTextHandler(io.Discard, nil)), MaxSamples: 10000000, Timeout: time.Minute, LookbackDelta: 5 * time.Minute, EnableAtModifier: true, EnableNegativeOffset: true, NoStepSubqueryIntervalFn: func(int64) int64 { return 60000 }})
	results := []result{}
	for _, r := range rs {
		if r.Lookback == 0 {
			r.Lookback = 300000
		}
		out := result{ID: r.ID, Query: r.Query, Rows: []row{}}
		var q promql.Query
		var err error
		opts := promql.NewPrometheusQueryOpts(false, time.Duration(r.Lookback)*time.Millisecond)
		if r.Step > 0 {
			q, err = engine.NewRangeQuery(context.Background(), source{d}, opts, r.Query, time.UnixMilli(r.Start), time.UnixMilli(r.End), time.Duration(r.Step)*time.Millisecond)
		} else {
			q, err = engine.NewInstantQuery(context.Background(), source{d}, opts, r.Query, time.UnixMilli(r.Time))
		}
		if err != nil {
			out.Kind = 4
			out.Error = err.Error()
			results = append(results, out)
			continue
		}
		v := q.Exec(context.Background())
		out.Warnings, out.Infos = v.Warnings.AsStrings(r.Query, 0, 0)
		if v.Err != nil {
			out.Kind = 4
			out.Error = v.Err.Error()
		} else {
			switch x := v.Value.(type) {
			case promql.Scalar:
				out.Kind = 1
				out.Rows = append(out.Rows, row{map[string]string{}, [][2]any{{x.T, val(x.V)}}})
			case promql.Vector:
				out.Kind = 2
				for _, s := range x {
					if s.H != nil {
						panic("native histogram requires separate typed probe")
					}
					out.Rows = append(out.Rows, row{lab(s.Metric), [][2]any{{s.T, val(s.F)}}})
				}
			case promql.Matrix:
				out.Kind = 3
				for _, s := range x {
					ps := [][2]any{}
					for _, p := range s.Floats {
						ps = append(ps, [2]any{p.T, val(p.F)})
					}
					out.Rows = append(out.Rows, row{lab(s.Metric), ps})
				}
			default:
				out.Kind = 4
				out.Error = fmt.Sprintf("unsupported reference result %T", x)
			}
		}
		q.Close()
		results = append(results, out)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"results": results, "reference": "Prometheus v3.15.0 / 5241a27fe3c6983549fccc32f6e65917408c63cd"}); err != nil {
		panic(err)
	}
}
