// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only reference capture, run from the pinned oracle module with downloads disabled.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"runtime"

	"github.com/prometheus/prometheus/promql/parser"
)

type request struct {
	ID    string `json:"id"`
	Input []byte `json:"input"`
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := []map[string]any{}
	p := parser.NewParser(parser.Options{ExperimentalDurationExpr: true})
	for _, r := range requests {
		expr, err := p.ParseExpr(string(r.Input))
		number, ok := expr.(*parser.NumberLiteral)
		if !ok {
			panic(fmt.Sprintf("case %s is not a number literal", r.ID))
		}
		diagnostics := [][]byte{}
		if err != nil {
			var list parser.ParseErrors
			if !errors.As(err, &list) {
				panic(err)
			}
			for _, diagnostic := range list {
				diagnostics = append(diagnostics, []byte(diagnostic.Err.Error()))
			}
		}
		results = append(results, map[string]any{
			"id": r.ID, "input": r.Input, "duration": number.Duration,
			"value_bits":  fmt.Sprintf("%016x", math.Float64bits(number.Val)),
			"nanoseconds": int64(math.Round(number.Val * 1e9)), "errors": diagnostics,
		})
	}
	output := map[string]any{
		"commit":     "5241a27fe3c6983549fccc32f6e65917408c63cd",
		"go_version": runtime.Version(), "architecture": runtime.GOARCH,
		"cases": results,
	}
	if err := json.NewEncoder(os.Stdout).Encode(output); err != nil {
		panic(err)
	}
}
