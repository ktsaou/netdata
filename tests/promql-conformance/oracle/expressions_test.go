// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"testing"

	"github.com/prometheus/prometheus/model/labels"
)

// Synthetic names cover the issue's expression shapes without copying infrastructure identities.
func TestRequiredSREExpressionShapes(t *testing.T) {
	const now int64 = 25200000
	d := dataset{Schema: schemaVersion}
	add := func(name string, value float64, ls ...string) {
		ls = append([]string{"__name__", name}, ls...)
		d.Series = append(d.Series, series{encodeLabels(labels.FromStrings(ls...)), []point{floatPoint(now, value)}})
	}
	for name, value := range map[string]float64{"traffic_in": 100, "traffic_out": 30, "subscriptions": 1} {
		add(name, value, "topic", "main", "namespace", "test")
	}
	add("volume_used", 95, "namespace", "test", "claim", "full")
	add("volume_capacity", 100, "namespace", "test", "claim", "full")
	add("volume_used", 90, "namespace", "test", "claim", "boundary")
	add("volume_capacity", 100, "namespace", "test", "claim", "boundary")
	add("hpa_spec", 1, "hpa", "one", "metric_name", "cpu")
	add("hpa_spec", 1, "hpa", "one", "metric_name", "memory")
	add("hpa_status", 1, "hpa", "one", "metric_name", "memory")
	add("hpa_current", 1, "hpa", "one")
	add("backlog", 100001, "namespace", "test", "topic", "main")
	add("classic_bucket", 1, "le", "1")
	add("classic_bucket", 2, "le", "2")
	add("classic_bucket", 2, "le", "+Inf")
	points := []point{}
	for i := int64(0); i <= 6; i++ {
		points = append(points, floatPoint(now-3600000+i*600000, float64(i)))
	}
	d.Series = append(d.Series, series{encodeLabels(labels.FromStrings("__name__", "task_example_failed")), points})
	if err := d.validate(); err != nil {
		t.Fatal(err)
	}
	tests := []struct {
		name, query string
		value       float64
		key, val    string
	}{
		{"absence", `absent(deployment_replicas{deployment="missing",namespace!="ignored"})`, 1, "deployment", "missing"},
		{"filtered backlog join", `(traffic_in{topic!~"ignored.*"} - traffic_out{topic!~"ignored.*"} > 50) and (subscriptions{topic!~"ignored.*"} > 0)`, 70, "topic", "main"},
		{"grouped capacity threshold", `sum by(namespace,claim)(volume_used) / sum by(namespace,claim)(volume_capacity) * 100 > 90`, 95, "claim", "full"},
		{"missing HPA status", `count(hpa_spec unless on(hpa,metric_name) hpa_status) by(hpa) and on(hpa) (max(hpa_current) by(hpa) > 0)`, 1, "hpa", "one"},
		{"time gated aggregation", `scalar(hour() == bool 7) * sum by(namespace,topic)(backlog) > 100000`, 100001, "topic", "main"},
		{"label replacement subquery", `sum by(name)(increase(label_replace({__name__=~"task_.*_failed"}, "name", "$1", "__name__", "(.*)")[1h:10m])) > 0`, 6, "name", "task_example_failed"},
		{"classic histogram", `histogram_quantile(0.5,classic_bucket)`, 1, "", ""},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			r := evaluate(d, instant(test.query, now))
			assertFloat(t, r, test.value)
			ls, err := r.Rows[0].Labels.decode()
			if err != nil {
				t.Fatal(err)
			}
			if test.key != "" && ls.Get(test.key) != test.val {
				t.Fatalf("result labels %s", ls)
			}
		})
	}
	for i, s := range d.Series {
		ls, _ := s.Labels.decode()
		if ls.Get("__name__") == "backlog" {
			d.Series[i].Points = append(d.Series[i].Points, floatPoint(now+3600000, 100001))
		}
	}
	assertFloat(t, evaluate(d, instant(`sum by(namespace,topic)(backlog)`, now+3600000)), 100001)
	r := evaluate(d, instant(`scalar(hour() == bool 7) * sum by(namespace,topic)(backlog) > 100000`, now+3600000))
	if r.Kind != "vector" || len(r.Rows) != 0 {
		t.Fatal("time-gated expression remains firing outside the hour")
	}
}
