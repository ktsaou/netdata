// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"encoding/json"
	"math"
	"reflect"
	"testing"

	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
)

func fixture(name string, points ...point) dataset {
	return dataset{schemaVersion, []series{{encodeLabels(labels.FromStrings("__name__", name, "instance", "test")), points}}}
}
func instant(query string, t int64) request {
	return request{ID: "test", Query: query, Profile: "server-default", Mode: "instant", Time: t}
}
func assertFloat(t *testing.T, r result, want float64) {
	t.Helper()
	if r.Error != nil {
		t.Fatalf("reference %s error: %s", r.Error.Phase, r.Error.Message)
	}
	if r.Error != nil || len(r.Rows) != 1 || len(r.Rows[0].Points) != 1 || r.Rows[0].Points[0].Float == nil {
		t.Fatalf("expected one float: %+v", r)
	}
	if got := r.Rows[0].Points[0].Float.value(); math.Float64bits(got) != math.Float64bits(want) {
		t.Fatalf("got %.17g (%x), want %.17g (%x)", got, math.Float64bits(got), want, math.Float64bits(want))
	}
}
func TestWirePreservesAllFloatBitsAndBytes(t *testing.T) {
	for _, raw := range []uint64{0, 1, 1 << 63, 0x7ff0000000000000, 0xfff0000000000000, 0x7ff0000000000002, 0x7ff8000000000123, 0xffffffffffffffff} {
		f := binary64(raw)
		p := point{Kind: "float", Float: &f, Time: 1000}
		d := dataset{schemaVersion, []series{{labelset{{[]byte("name"), []byte{0xff, 0, 0xfe}}}, []point{p}}}}
		b, err := json.Marshal(d)
		if err != nil {
			t.Fatal(err)
		}
		var restored dataset
		if err = json.Unmarshal(b, &restored); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(d, restored) {
			t.Fatalf("lossy round trip for %x", raw)
		}
	}
}
func TestHistogramCopiesAndTypedResults(t *testing.T) {
	h := &histogram.FloatHistogram{CounterResetHint: histogram.GaugeType, Schema: 0, Count: 2, Sum: 1.5,
		PositiveSpans: []histogram.Span{{Offset: 0, Length: 1}}, PositiveBuckets: []float64{2}}
	wire := encodeFloatHistogram(h)
	h.PositiveBuckets[0] = 9
	if wire.PositiveBuckets[0].value() != 2 {
		t.Fatal("histogram aliases source")
	}
	d := fixture("h", point{Time: 600000, Kind: "float_histogram", FloatHistogram: wire})
	if err := d.validate(); err != nil {
		t.Fatal(err)
	}
	for q, want := range map[string]float64{"histogram_count(h)": 2, "histogram_sum(h)": 1.5, "histogram_avg(h)": 0.75} {
		assertFloat(t, evaluate(d, instant(q, 600000)), want)
	}
	r := evaluate(d, instant("h", 600000))
	if r.Kind != "vector" || r.Rows[0].Points[0].Kind != "float_histogram" {
		t.Fatalf("typed vector lost: %+v", r)
	}
	if r.Rows[0].Points[0].FloatHistogram.Count.value() != 2 {
		t.Fatal("histogram output count")
	}
	integer := &histogram.Histogram{Schema: 0, Count: 2, Sum: 1.5, PositiveSpans: []histogram.Span{{Offset: 0, Length: 1}}, PositiveBuckets: []int64{2}}
	d = fixture("ih", point{Time: 600000, Kind: "histogram", Histogram: encodeIntegerHistogram(integer)})
	assertFloat(t, evaluate(d, instant("histogram_count(ih)", 600000)), 2)
}
func TestMixedTimelineAndStaleness(t *testing.T) {
	h := &histogram.FloatHistogram{Schema: 0, Count: 2, Sum: 1.5, PositiveSpans: []histogram.Span{{Offset: 0, Length: 1}}, PositiveBuckets: []float64{2}}
	d := fixture("m", floatPoint(1000, 1), point{Time: 2000, Kind: "float_histogram", FloatHistogram: encodeFloatHistogram(h)}, floatPoint(3000, 3))
	r := evaluate(d, instant("m[5s]", 3000))
	if r.Kind != "matrix" || len(r.Rows) != 1 || len(r.Rows[0].Points) != 3 {
		t.Fatalf("lost mixed matrix: %+v", r)
	}
	if r.Rows[0].Points[1].Kind != "float_histogram" {
		t.Fatal("lost histogram timeline")
	}
	r = evaluate(d, instant("rate(m[5s])", 3000))
	if r.Kind != "vector" || len(r.Rows) != 0 || len(r.Warnings) != 1 {
		t.Fatalf("mixed rate warning not preserved: %+v", r)
	}
	stale := floatPoint(2000, math.Float64frombits(0x7ff0000000000002))
	d = fixture("m", floatPoint(1000, 1), stale)
	r = evaluate(d, instant("absent(m)", 2000))
	assertFloat(t, r, 1)
	d = fixture("m", floatPoint(2000, math.Float64frombits(0x7ff8000000000123)))
	r = evaluate(d, instant("m", 2000))
	if len(r.Rows) != 1 || !math.IsNaN(r.Rows[0].Points[0].Float.value()) {
		t.Fatal("ordinary NaN treated as stale")
	}
}
func TestStringsProfilesAndDiagnostics(t *testing.T) {
	d := dataset{Schema: schemaVersion}
	r := evaluate(d, instant(`"\xff\x00"`, 0))
	if r.Kind != "string" || !reflect.DeepEqual(r.Rows[0].Points[0].String, []byte{0xff, 0}) {
		t.Fatalf("lossy string result: %+v", r)
	}
	assertFloat(t, evaluate(d, instant("time()", 25200000)), 25200)
	assertFloat(t, evaluate(d, instant("hour()", 25200000)), 7)
	for _, query := range []string{"m[5m+1m]", "m offset -1h", "m @ start()", "sum(rate(recorded:metric:5m[5m]))"} {
		a := analyze(instant(query, 0))
		if a.Error != nil {
			t.Fatalf("default syntax %q rejected: %+v", query, a.Error)
		}
	}
	q := instant("sort_by_label(m,\"instance\")", 0)
	if a := analyze(q); a.Error == nil {
		t.Fatal("experimental function accepted by default")
	}
	q.Profile = "upstream-tests"
	if a := analyze(q); a.Error != nil {
		t.Fatalf("experimental profile rejected: %+v", a.Error)
	}
	r = evaluate(d, instant("rate(1)", 0))
	if r.Error == nil || r.Error.Phase != "construction" || len(r.Error.ParseErrors) == 0 {
		t.Fatalf("type-check diagnostic lost: %+v", r)
	}
	r = evaluate(d, instant("sum(", 0))
	if r.Error == nil || len(r.Error.ParseErrors) == 0 || r.Error.ParseErrors[0].Start < 0 {
		t.Fatal("syntax position lost")
	}
}
func TestSampleTimestampAndRangeContext(t *testing.T) {
	d := fixture("m", floatPoint(1000, 1), floatPoint(2000, 2), floatPoint(3000, 3))
	assertFloat(t, evaluate(d, instant("timestamp(m)", 2500)), 2)
	r := instant("m @ start()", 0)
	r.Mode = "range"
	r.Start = 2000
	r.End = 3000
	r.Step = 1000
	out := evaluate(d, r)
	if out.Kind != "matrix" || len(out.Rows) != 1 || len(out.Rows[0].Points) != 2 {
		t.Fatalf("range result: %+v", out)
	}
	for i, p := range out.Rows[0].Points {
		if p.Time != int64(2000+i*1000) || p.Float.value() != 2 {
			t.Fatal("range @ start changed with evaluation step")
		}
	}
	assertFloat(t, evaluate(d, instant("m offset -1s", 2000)), 3)
}
func TestThresholdAndBoolAreExact(t *testing.T) {
	d := fixture("m", floatPoint(0, 0.6))
	assertFloat(t, evaluate(d, instant("m >= 0.6", 0)), 0.6)
	assertFloat(t, evaluate(d, instant("m >= bool 0.6", 0)), 1)
	d = fixture("m", floatPoint(0, math.Nextafter(0.6, 0)))
	r := evaluate(d, instant("m >= 0.6", 0))
	if r.Kind != "vector" || len(r.Rows) != 0 {
		t.Fatal("one-bit threshold difference ignored")
	}
	assertFloat(t, evaluate(d, instant("m >= bool 0.6", 0)), 0)
}
func TestRateOperationOrderThresholdRegression(t *testing.T) {
	// The retained B049 fixture exercises extrapolation where operation order changes one bit.
	d := fixture("counter_total", floatPoint(480000, 100), floatPoint(600000, 220))
	assertFloat(t, evaluate(d, instant("rate(counter_total[5m])", 600000)), 0.6)
	assertFloat(t, evaluate(d, instant("rate(counter_total[5m]) >= 0.6", 600000)), 0.6)
	assertFloat(t, evaluate(d, instant("rate(counter_total[5m]) >= bool 0.6", 600000)), 1)
}
func TestRejectsAmbiguousInput(t *testing.T) {
	d := fixture("m", floatPoint(1, 1), floatPoint(1, 2))
	if d.validate() == nil {
		t.Fatal("duplicate timestamp accepted")
	}
	d = fixture("m", point{Time: 1, Kind: "float"})
	if d.validate() == nil {
		t.Fatal("missing numeric payload accepted")
	}
	d = fixture("m", floatPoint(1, 1))
	d.Series = append(d.Series, d.Series[0])
	if d.validate() == nil {
		t.Fatal("duplicate labelset accepted")
	}
	f := binary64(0)
	if json.Unmarshal([]byte(`"NaN"`), &f) == nil {
		t.Fatal("lossy float input accepted")
	}
}
