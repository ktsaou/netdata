// SPDX-License-Identifier: GPL-3.0-or-later
// Query contexts qualify the pinned engine's highest invariant boundaries.
package main

import (
	"encoding/json"
	"os"
	"time"

	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"netdata-promql-conformance-oracle/codec"
	"netdata-promql-conformance-oracle/wire"
)

func main() {
	data, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var requests []wire.Request
	if err := json.Unmarshal(data, &requests); err != nil {
		panic(err)
	}
	results := []map[string]any{}
	for _, request := range requests {
		scope := map[string]any{}
		row := map[string]any{"id": request.ID, "scope": scope}
		results = append(results, row)
		expr, err := parser.NewParser(parser.Options{EnableExperimentalFunctions: true,
			ExperimentalDurationExpr: true}).ParseExpr(request.Query)
		if err != nil {
			scope["error"] = codec.Error("parse", err)
			continue
		}
		start, end := time.UnixMilli(request.Start), time.UnixMilli(request.End)
		step := time.Duration(request.Step) * time.Millisecond
		if request.Mode == "instant" {
			start = time.UnixMilli(request.Time)
			end, step = start, 0
		}
		expr, err = promql.PreprocessExpr(expr, start, end, step)
		if err != nil {
			scope["error"] = codec.Error("preprocess", err)
			continue
		}
		boundaries := [][2]int{}
		parser.Inspect(expr, func(node parser.Node, _ []parser.Node) error {
			if node != nil {
				if _, ok := node.(*parser.StepInvariantExpr); ok {
					span := node.PositionRange()
					boundaries = append(boundaries, [2]int{int(span.Start), int(span.End)})
				}
			}
			return nil
		})
		scope["boundaries"] = boundaries
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"commit": wire.ReferenceCommit, "results": results,
	}); err != nil {
		panic(err)
	}
}
