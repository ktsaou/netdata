// SPDX-License-Identifier: GPL-3.0-or-later
// Extract untouched test bodies and operation call sites from the pinned source.
package main

import (
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
)

func main() {
	if len(os.Args) != 2 {
		panic("expected pinned Prometheus source root")
	}
	definitions := map[string]any{}
	for name, source := range map[string]string{
		"TestBucketFractionBelow":                     "model/histogram/generic_test.go",
		"TestFloatHistogramTrimBucketsInPlace":        "model/histogram/float_histogram_test.go",
		"TestTrimBuckets_HistogramFractionCrossCheck": "promql/quantile_test.go",
	} {
		raw, err := os.ReadFile(filepath.Join(os.Args[1], source))
		if err != nil {
			panic(err)
		}
		files := token.NewFileSet()
		file, err := parser.ParseFile(files, source, raw, 0)
		if err != nil {
			panic(err)
		}
		for _, declaration := range file.Decls {
			function, ok := declaration.(*ast.FuncDecl)
			if !ok || function.Name.Name != name {
				continue
			}
			if _, exists := definitions[name]; exists {
				panic("duplicate trim source definition")
			}
			lines := []int{}
			ast.Inspect(function, func(node ast.Node) bool {
				call, ok := node.(*ast.CallExpr)
				if !ok {
					return true
				}
				selector, ok := call.Fun.(*ast.SelectorExpr)
				if ok && (selector.Sel.Name == "TrimBuckets" || selector.Sel.Name == "FractionBelow") {
					lines = append(lines, files.Position(call.Pos()).Line)
				}
				return true
			})
			start, end := files.Position(function.Pos()), files.Position(function.End())
			definitions[name] = map[string]any{"source": source, "line": start.Line,
				"body": string(raw[start.Offset:end.Offset]), "call_lines": lines}
		}
	}
	if len(definitions) != 3 {
		panic("changed trim source definitions")
	}
	if err := json.NewEncoder(os.Stdout).Encode(definitions); err != nil {
		panic(err)
	}
}
