// SPDX-License-Identifier: GPL-3.0-or-later
// Compile original Go declarations instead of interpreting their float expressions.
package main

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/format"
	"go/parser"
	"go/token"
	"os"
	"strconv"
)

func main() {
	if len(os.Args) != 3 {
		panic("expected original ftoa_test.go and output Go file")
	}
	raw, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	fset := token.NewFileSet()
	file, err := parser.ParseFile(fset, "ftoa_test.go", raw, 0)
	if err != nil {
		panic(err)
	}
	wanted := map[string]bool{"ftoaTest": false, "fdiv": false, "below1e23": false, "above1e23": false, "ftoatests": false}
	var out bytes.Buffer
	out.WriteString("// Copyright 2009 The Go Authors. All rights reserved.\n// BSD license; see the captured original LICENSE.\npackage main\nimport \"math\"\n")
	var rows *ast.CompositeLit
	for _, decl := range file.Decls {
		keep := false
		switch node := decl.(type) {
		case *ast.FuncDecl:
			if _, ok := wanted[node.Name.Name]; ok {
				wanted[node.Name.Name], keep = true, true
			}
		case *ast.GenDecl:
			for _, spec := range node.Specs {
				switch spec := spec.(type) {
				case *ast.TypeSpec:
					if _, ok := wanted[spec.Name.Name]; ok {
						wanted[spec.Name.Name], keep = true, true
					}
				case *ast.ValueSpec:
					for _, name := range spec.Names {
						if _, ok := wanted[name.Name]; ok {
							wanted[name.Name], keep = true, true
							if name.Name == "ftoatests" {
								rows = spec.Values[0].(*ast.CompositeLit)
							}
						}
					}
				}
			}
		}
		if keep {
			if err := format.Node(&out, fset, decl); err != nil {
				panic(err)
			}
			out.WriteByte('\n')
		}
	}
	for name, found := range wanted {
		if !found {
			panic("missing original declaration: " + name)
		}
	}
	if rows == nil || len(rows.Elts) != 148 {
		panic("original float formatting table changed")
	}
	out.WriteString("var originalLines = []int{")
	for _, row := range rows.Elts {
		fmt.Fprintf(&out, "%d,", fset.Position(row.Pos()).Line)
	}
	out.WriteString("}\nvar originalExpressions = []string{\n")
	for _, row := range rows.Elts {
		value := row.(*ast.CompositeLit).Elts[0]
		start, end := fset.Position(value.Pos()).Offset, fset.Position(value.End()).Offset
		fmt.Fprintf(&out, "%s,\n", strconv.Quote(string(raw[start:end])))
	}
	out.WriteString("}\n")
	generated, err := format.Source(out.Bytes())
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(os.Args[2], generated, 0600); err != nil {
		panic(err)
	}
}
