// SPDX-License-Identifier: GPL-3.0-or-later
// Keep source owners and original assertion bodies independent of candidate code.
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
	if len(os.Args) != 3 {
		panic("expected pinned Prometheus and Go roots")
	}
	owners := map[string][]string{
		"promql/quantile.go":         {"BucketQuantile", "BucketFraction", "coalesceBuckets", "ensureMonotonicAndIgnoreSmallDeltas", "smallDeltaTolerance"},
		"promql/quantile_test.go":    {"TestBucketQuantile_ForcedMonotonicity"},
		"util/almost/almost.go":      {"Equal", "minNormal"},
		"util/almost/almost_test.go": {"TestEqual"},
		"src/slices/sort.go":         {"SortFunc", "sortedHint", "unknownHint", "xorshift", "Next", "nextPowerOfTwo"},
		"src/slices/zsortanyfunc.go": {"insertionSortCmpFunc", "siftDownCmpFunc", "heapSortCmpFunc", "pdqsortCmpFunc", "partitionCmpFunc", "partitionEqualCmpFunc", "partialInsertionSortCmpFunc", "breakPatternsCmpFunc", "choosePivotCmpFunc", "order2CmpFunc", "medianCmpFunc", "medianAdjacentCmpFunc", "reverseRangeCmpFunc"},
		"src/sort/search.go":         {"Search"},
	}
	definitions := map[string]any{}
	for source, names := range owners {
		root := os.Args[1]
		if source[:4] == "src/" {
			root = os.Args[2]
		}
		raw, err := os.ReadFile(filepath.Join(root, source))
		if err != nil {
			panic(err)
		}
		files := token.NewFileSet()
		file, err := parser.ParseFile(files, source, raw, 0)
		if err != nil {
			panic(err)
		}
		wanted := map[string]bool{}
		for _, name := range names {
			wanted[name] = true
		}
		record := func(name string, node ast.Node) {
			if !wanted[name] {
				return
			}
			if _, exists := definitions[name]; exists {
				panic("duplicate classic source owner: " + source + ":" + name)
			}
			start, end := files.Position(node.Pos()), files.Position(node.End())
			calls := []int{}
			ast.Inspect(node, func(node ast.Node) bool {
				if call, ok := node.(*ast.CallExpr); ok {
					if ident, ok := call.Fun.(*ast.Ident); ok && (ident.Name == "BucketQuantile" || ident.Name == "Equal") {
						calls = append(calls, files.Position(call.Pos()).Line)
					}
				}
				return true
			})
			definitions[name] = map[string]any{"source": source, "line": start.Line, "body": string(raw[start.Offset:end.Offset]), "call_lines": calls}
		}
		for _, declaration := range file.Decls {
			switch declaration := declaration.(type) {
			case *ast.FuncDecl:
				if declaration.Recv == nil || declaration.Name.Name == "Next" {
					record(declaration.Name.Name, declaration)
				}
			case *ast.GenDecl:
				for _, spec := range declaration.Specs {
					switch spec := spec.(type) {
					case *ast.ValueSpec:
						for _, name := range spec.Names {
							record(name.Name, declaration)
						}
					case *ast.TypeSpec:
						record(spec.Name.Name, declaration)
					}
				}
			}
		}
		for _, name := range names {
			if _, found := definitions[name]; !found {
				panic("missing classic source owner: " + name)
			}
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(definitions); err != nil {
		panic(err)
	}
}
