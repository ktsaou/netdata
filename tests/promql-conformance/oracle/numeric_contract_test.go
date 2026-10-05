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
		"m > 1", "m + 1", "0.1", "avg(m > bool 1)", "scalar(m)", "round(m, 0.1)", "histogram_count(m)",
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

func TestExactNumericIntegerFunctionsAndFill(t *testing.T) {
	for query, exact := range map[string]bool{
		"sgn(m)": true, "ceil(m)": true, "floor(m)": true,
		"days_in_month(m)": true, "day_of_month(m)": true, "day_of_week()": true,
		"day_of_year(m)": true, "hour(m)": true, "minute(m)": true, "month(m)": true, "year(m)": true,
		"1": true, "count(m) + 1": true, "round(m)": true, "round(m, -1)": true,
		"round(m, 0.1)": false, "round(m, 3)": false, "round(m, scalar(count(n)))": false,
		"count(m) + fill(0.1) count(n)":                    false,
		"count(m) + fill(0) count(n)":                      true,
		"count(m) + fill_left(0.1) fill_right(0) count(n)": false,
		"count(m) > fill(0.1) count(n)":                    false,
		"count(m) > fill_left(0.1) count(n)":               false,
		"count(m) > fill_right(0.1) count(n)":              true,
		"count(m) > bool fill(0.1) count(n)":               true,
		"count(m) + fill(NaN) count(n)":                    true,
	} {
		request := instant(query, 0)
		request.Profile = "upstream-tests"
		out := analyze(request)
		if out.Error != nil || out.ExactNumeric != exact {
			t.Errorf("wrong numeric contract for %q: want %t, got %+v", query, exact, out)
		}
	}
}
