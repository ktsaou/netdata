package main

import (
	"reflect"
	"testing"

	"github.com/prometheus/prometheus/tsdb/chunkenc"
)

func TestNativeHistogramReference(t *testing.T) {
	probe := nativeHistogramReference()
	got := probe["results"].([]result)
	values := map[string]float64{"histogram_count": 2, "histogram_sum": 1.5, "histogram_avg": 0.75}
	for _, r := range got {
		want := result{ID: r.ID, Query: r.ID + "(native_hist)", Kind: 2,
			Rows: []row{singleRow(map[string]string{"case": "hist"}, 600000, values[r.ID])}}
		if !reflect.DeepEqual(r.Rows, want.Rows) || r.Kind != want.Kind || r.Error != "" || len(r.Warnings)+len(r.Infos) != 0 {
			t.Fatalf("%s: got %#v, want %#v", r.ID, r, want)
		}
	}
	if len(got) != len(values) {
		t.Fatal("native histogram probe operations missing")
	}
}

func TestNativeHistogramSampleOwnership(t *testing.T) {
	source := histogramSample{histogramFixture()}
	copy := source.Copy()
	view := source.FH()
	source.h.PositiveBuckets[0] = 0
	source.h.Sum = 0
	if copy.Type() != chunkenc.ValFloatHistogram || copy.FH().PositiveBuckets[0] != 2 || copy.FH().Sum != 1.5 ||
		view.PositiveBuckets[0] != 2 || view.Sum != 1.5 {
		t.Fatal("typed sample handoff must own bucket storage")
	}
}
