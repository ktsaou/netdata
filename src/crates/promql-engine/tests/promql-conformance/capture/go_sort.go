// SPDX-License-Identifier: GPL-3.0-or-later
// Observe original Go slices.SortFunc with Prometheus's literal bound comparator.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"slices"
	"strconv"
)

type request struct {
	ID     string   `json:"id"`
	Bounds []string `json:"bounds"`
}

type item struct {
	Bound   string `json:"bound"`
	Ordinal int    `json:"ordinal"`
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	output := json.NewEncoder(os.Stdout)
	for _, request := range requests {
		data := make([]item, len(request.Bounds))
		for i, bound := range request.Bounds {
			data[i] = item{bound, i}
		}
		comparisons, trace := uint64(0), uint64(14695981039346656037)
		slices.SortFunc(data, func(a, b item) int {
			comparisons++
			for _, ordinal := range [...]uint64{uint64(a.Ordinal), uint64(b.Ordinal)} {
				for shift := 0; shift < 64; shift += 8 {
					trace ^= (ordinal >> shift) & 255
					trace *= 1099511628211
				}
			}
			x, err := strconv.ParseUint(a.Bound, 16, 64)
			if err != nil {
				panic(err)
			}
			y, err := strconv.ParseUint(b.Bound, 16, 64)
			if err != nil {
				panic(err)
			}
			// prometheus/prometheus @5241a27fe3c6983549fccc32f6e65917408c63cd
			// promql/quantile.go:123-132,523-532.
			if math.Float64frombits(x) < math.Float64frombits(y) {
				return -1
			}
			if math.Float64frombits(x) > math.Float64frombits(y) {
				return +1
			}
			return 0
		})
		if err := output.Encode(map[string]any{"id": request.ID, "actual": map[string]any{
			"items": data, "comparisons": comparisons, "trace": fmt.Sprintf("%016x", trace),
		}}); err != nil {
			panic(err)
		}
	}
}
