// SPDX-License-Identifier: GPL-3.0-or-later
// Observe untouched prometheus/prometheus @5241a27fe3c6983549fccc32f6e65917408c63cd
// promql/quantile.go:105-169,520-633,668-707. No copied numeric implementation.
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"os"
	"runtime"
	"strconv"

	"github.com/prometheus/prometheus/promql"
)

type bucket struct {
	Bound string `json:"upper_bound"`
	Count string `json:"count"`
}

type operation struct {
	Kind  string `json:"kind"`
	Q     string `json:"quantile"`
	Lower string `json:"lower"`
	Upper string `json:"upper"`
}

type request struct {
	ID        string    `json:"id"`
	Buckets   []bucket  `json:"buckets"`
	Operation operation `json:"operation"`
}

func number(s string) float64 {
	x, err := strconv.ParseUint(s, 16, 64)
	if err != nil || len(s) != 16 {
		panic("invalid binary64")
	}
	return math.Float64frombits(x)
}

func bits(x float64) string { return fmt.Sprintf("%016x", math.Float64bits(x)) }

func main() {
	if runtime.Version() != "go1.27.1" || runtime.GOARCH != "amd64" || runtime.GOOS != "linux" {
		panic("invalid runtime")
	}
	decoder := json.NewDecoder(bufio.NewReader(os.Stdin))
	decoder.DisallowUnknownFields()
	encoder := json.NewEncoder(os.Stdout)
	for {
		var r request
		if err := decoder.Decode(&r); err != nil {
			if err == io.EOF {
				break
			}
			panic(err)
		}
		buckets := make(promql.Buckets, len(r.Buckets))
		for i, b := range r.Buckets {
			buckets[i] = promql.Bucket{UpperBound: number(b.Bound), Count: number(b.Count)}
		}
		var result any
		panicked := true
		func() {
			defer func() { recover() }()
			var value float64
			var notes any
			switch r.Operation.Kind {
			case "quantile":
				v, forced, fixed, min, max, decrease := promql.BucketQuantile(number(r.Operation.Q), buckets)
				value = v
				notes = map[string]any{"forced_monotonicity": forced, "fixed_precision": fixed,
					"min_bound": bits(min), "max_bound": bits(max), "max_decrease": bits(decrease)}
			case "fraction":
				value = promql.BucketFraction(number(r.Operation.Lower), number(r.Operation.Upper), buckets)
			default:
				panic("unknown operation")
			}
			reciprocal := 1 / value
			result = map[string]any{"value": bits(value), "quantile": notes, "reciprocal": bits(reciprocal),
				"negative_reciprocal_infinite": reciprocal == math.Inf(-1),
				"positive_reciprocal_infinite": reciprocal == math.Inf(1),
				"less_than_positive_infinity":  value < math.Inf(1)}
			panicked = false
		}()
		after := make([]bucket, len(buckets))
		for i, b := range buckets {
			after[i] = bucket{bits(b.UpperBound), bits(b.Count)}
		}
		if err := encoder.Encode(map[string]any{"id": r.ID,
			"actual": map[string]any{"panicked": panicked, "result": result, "input_after": after}}); err != nil {
			panic(err)
		}
	}
}
