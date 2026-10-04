// SPDX-License-Identifier: GPL-3.0-or-later
// Preserve exact Go declarations and original test bodies alongside observations.
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
		panic("expected pinned Go source root")
	}
	raw, err := os.ReadFile(filepath.Join(os.Args[1], "src/math/all_test.go"))
	if err != nil {
		panic(err)
	}
	files := token.NewFileSet()
	file, err := parser.ParseFile(files, "src/math/all_test.go", raw, 0)
	if err != nil {
		panic(err)
	}
	definitions := map[string]any{}
	wanted := map[string]bool{}
	for _, name := range []string{"vf", "frexp", "vffrexpSC", "frexpSC", "vffrexpBC", "frexpBC", "log", "log2", "vflogSC", "logSC", "sqrt", "vfsqrtSC", "sqrtSC", "TestFrexp", "TestLog", "TestLog2", "TestSqrt", "tolerance", "veryclose", "alike"} {
		wanted[name] = true
	}
	record := func(name string, declaration ast.Node) {
		if !wanted[name] {
			return
		}
		if _, exists := definitions[name]; exists {
			panic("duplicate source definition: " + name)
		}
		start, end := files.Position(declaration.Pos()), files.Position(declaration.End())
		definitions[name] = map[string]any{"line": start.Line, "source": "src/math/all_test.go", "body": string(raw[start.Offset:end.Offset])}
	}
	for _, declaration := range file.Decls {
		switch declaration := declaration.(type) {
		case *ast.FuncDecl:
			record(declaration.Name.Name, declaration)
		case *ast.GenDecl:
			for _, spec := range declaration.Specs {
				if value, ok := spec.(*ast.ValueSpec); ok {
					for _, name := range value.Names {
						record(name.Name, declaration)
					}
				}
			}
		}
	}
	if len(definitions) != len(wanted) {
		panic("changed Go numerical definitions")
	}
	if err := json.NewEncoder(os.Stdout).Encode(definitions); err != nil {
		panic(err)
	}
}
