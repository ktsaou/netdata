// SPDX-License-Identifier: GPL-3.0-or-later
package main

import "testing"

func TestExactNumericReferenceContract(t *testing.T) {
	for _, query := range []string{
		"m == bool 1", "-(m >= bool 1)", "count(m)", "group(m)", "count_values(\"x\", m)",
		"sum(m == bool 1)", "count(m) + count(n)", "count(m) > 1", "sort(count(m))",
		"label_replace(count(m), \"x\", \"y\", \"z\", \".*\")", "scalar(count(m))", "vector(scalar(count(m)))",
		"count_over_time(m[1m])", "present_over_time(m[1m])", "changes(m[1m])", "resets(m[1m])", "absent(m)",
		"first_over_time((m > bool 1)[1m:])", "sum_over_time((count(m))[1m:])",
	} {
		out := analyze(instant(query, 0))
		if out.Error != nil || !out.ExactNumeric {
			t.Errorf("discrete contract lost for %q: %+v", query, out)
		}
	}
	for _, query := range []string{
		"m > 1", "m + 1", "1", "avg(m > bool 1)", "scalar(m)", "round(m)", "histogram_count(m)",
		"count(m) / count(n)", "count(m) ^ -count(n)", "count(m) * m", "sum_over_time(m[1m])",
		"m{description=\"== bool\"}", "count(m) or m", "min_over_time(m[1m])", "time()", "timestamp(m)",
		"scalar(count(m)) > m",
	} {
		out := analyze(instant(query, 0))
		if out.Error != nil || out.ExactNumeric {
			t.Errorf("numeric contract tightened for %q: %+v", query, out)
		}
	}
}
