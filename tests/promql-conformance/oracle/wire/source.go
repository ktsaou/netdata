// SPDX-License-Identifier: GPL-3.0-or-later
package wire

import (
	"sort"

	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"github.com/prometheus/prometheus/tsdb/chunks"
	"github.com/prometheus/prometheus/util/annotations"
)

type sample struct{ Point }

func (s sample) T() int64  { return s.Time }
func (s sample) ST() int64 { return s.StartTime }
func (s sample) Type() chunkenc.ValueType {
	switch s.Kind {
	case "float":
		return chunkenc.ValFloat
	case "histogram":
		return chunkenc.ValHistogram
	default:
		return chunkenc.ValFloatHistogram
	}
}
func (s sample) F() float64              { return s.Float.Value() }
func (s sample) H() *histogram.Histogram { return s.Histogram.Decode() }
func (s sample) FH() *histogram.FloatHistogram {
	if s.Histogram != nil {
		return s.Histogram.Decode().ToFloat(nil)
	}
	return s.FloatHistogram.Decode()
}
func (s sample) Copy() chunks.Sample {
	copy := s
	if s.Float != nil {
		f := *s.Float
		copy.Float = &f
	}
	if s.Histogram != nil {
		copy.Histogram = EncodeIntegerHistogram(s.H())
	}
	if s.FloatHistogram != nil {
		copy.FloatHistogram = EncodeFloatHistogram(s.FH())
	}
	return copy
}

type seriesSet struct {
	data  []storage.Series
	index int
}

func (s *seriesSet) Next() bool                      { s.index++; return s.index < len(s.data) }
func (s *seriesSet) At() storage.Series              { return s.data[s.index] }
func (*seriesSet) Err() error                        { return nil }
func (*seriesSet) Warnings() annotations.Annotations { return nil }

type Source struct{ Dataset }

func (q Source) Querier(mint, maxt int64) (storage.Querier, error) {
	return &storage.MockQuerier{SelectMockFunction: func(sorted bool, _ *storage.SelectHints, ms ...*labels.Matcher) storage.SeriesSet {
		out := []storage.Series{}
		for _, s := range q.Series {
			ls, err := s.Labels.Decode()
			if err != nil {
				panic(err)
			}
			matched := true
			for _, m := range ms {
				if !m.Matches(ls.Get(m.Name)) {
					matched = false
					break
				}
			}
			if !matched {
				continue
			}
			points := []chunks.Sample{}
			for _, p := range s.Points {
				if p.Time >= mint && p.Time <= maxt {
					points = append(points, sample{p})
				}
			}
			out = append(out, storage.NewListSeries(ls, points))
		}
		if sorted {
			sort.Slice(out, func(i, j int) bool { return labels.Compare(out[i].Labels(), out[j].Labels()) < 0 })
		}
		return &seriesSet{out, -1}
	}}, nil
}
