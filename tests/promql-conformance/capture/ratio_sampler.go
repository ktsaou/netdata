// SPDX-License-Identifier: GPL-3.0-or-later
// Test facade invokes the pinned sampler; primitive normalization mirrors engine.go:4835-4839.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"runtime"
	"strconv"
	"unicode"

	"github.com/prometheus/prometheus/promql"
	"netdata-promql-conformance-oracle/wire"
)

type samplerInput struct {
	ID     string        `json:"id"`
	Labels wire.Labelset `json:"labels"`
	Hash   *string       `json:"hash"`
}
type samplerThreshold struct {
	Ratio    wire.Binary64 `json:"ratio"`
	Selected bool          `json:"selected"`
}
type samplerOutput struct {
	ID         string             `json:"id"`
	Hash       string             `json:"hash"`
	Offset     wire.Binary64      `json:"offset"`
	Thresholds []samplerThreshold `json:"thresholds"`
}

func main() {
	var inputs []samplerInput
	if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil {
		panic(err)
	}
	sampler := promql.NewHashRatioSampler()
	rows := make([]samplerOutput, 0, len(inputs))
	for _, input := range inputs {
		var hash uint64
		var offset float64
		var sample promql.Sample
		if input.Hash == nil {
			if input.Labels == nil {
				panic("missing label input")
			}
			metric, err := input.Labels.Decode()
			if err != nil {
				panic(err)
			}
			sample.Metric = metric
			hash = metric.Hash()
			offset = sampler.SampleOffset(&metric)
		} else {
			if input.Labels != nil {
				panic("ambiguous hash input")
			}
			var err error
			hash, err = strconv.ParseUint(*input.Hash, 16, 64)
			if err != nil {
				panic(err)
			}
			offset = float64(hash) / float64(math.MaxUint64)
		}
		ratios := []float64{0, math.Float64frombits(1 << 63), 0.5, -0.5, 1, -1,
			math.Inf(1), math.Inf(-1), math.NaN()}
		for _, boundary := range []float64{offset, offset - 1} {
			ratios = append(ratios, math.Nextafter(boundary, math.Inf(-1)), boundary,
				math.Nextafter(boundary, math.Inf(1)))
		}
		output := samplerOutput{input.ID, fmt.Sprintf("%016x", hash), wire.Number(offset),
			make([]samplerThreshold, 0, len(ratios))}
		for _, ratio := range ratios {
			selected := sampler.AddRatioSampleWithOffset(ratio, offset)
			if input.Hash == nil && selected != sampler.AddRatioSample(ratio, &sample) {
				panic("sampler entry points disagree")
			}
			output.Thresholds = append(output.Thresholds, samplerThreshold{wire.Number(ratio), selected})
		}
		rows = append(rows, output)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema_version": 1,
		"commit": wire.ReferenceCommit, "reference": wire.ReferenceModule,
		"go_version": runtime.Version(), "go_os": runtime.GOOS, "go_arch": runtime.GOARCH,
		"unicode_version": unicode.Version, "cases": rows}); err != nil {
		panic(err)
	}
}
