// SPDX-License-Identifier: GPL-3.0-or-later
// Extract exact declaration bytes; expectations remain computed by pinned Go.
package main

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"strconv"
)

type table struct {
	Test       string   `json:"test"`
	File       string   `json:"file"`
	Mode       string   `json:"mode"`
	Cases      int      `json:"cases"`
	Line       int      `json:"line"`
	CaseLines  []int    `json:"case_lines"`
	SourceHash string   `json:"source_sha256"`
	Prefix     string   `json:"prefix"`
	Variable   string   `json:"variable"`
	CaseKeys   []string `json:"case_keys"`
}

func main() {
	if len(os.Args) != 2 {
		panic("expected pinned histogram source directory")
	}
	tables := []table{
		{Test: "TestFloatHistogramMul", File: "float_histogram_test.go", Mode: "float-mul", Cases: 9},
		{Test: "TestFloatHistogramDiv", File: "float_histogram_test.go", Mode: "float-div", Cases: 8},
		{Test: "TestFloatHistogramCopy", File: "float_histogram_test.go", Mode: "float-copy", Cases: 4},
		{Test: "TestFloatHistogramCopyTo", File: "float_histogram_test.go", Mode: "float-copy-to", Cases: 4},
		{Test: "TestHistogramCopy", File: "histogram_test.go", Mode: "integer-copy", Cases: 4},
		{Test: "TestHistogramCopyTo", File: "histogram_test.go", Mode: "integer-copy-to", Cases: 4},
		{Test: "TestHistogramToFloat", File: "histogram_test.go", Mode: "integer-to-float", Cases: 3},
		{Test: "TestCustomBucketsHistogramToFloat", File: "histogram_test.go", Mode: "integer-to-float", Cases: 3},
		{Test: "TestFloatHistogramCompact", File: "float_histogram_test.go", Mode: "float-compact", Cases: 24},
		{Test: "TestFloatHistogramCopyToSchema", File: "float_histogram_test.go", Mode: "float-copy-to-schema", Cases: 3},
		{Test: "TestFloatHistogramReduceResolution", File: "float_histogram_test.go", Mode: "float-reduce-resolution", Cases: 4, Variable: "tcs"},
	}
	for i := range tables {
		row := &tables[i]
		if row.Variable == "" {
			row.Variable = "cases"
		}
		raw, err := os.ReadFile(filepath.Join(os.Args[1], row.File))
		if err != nil {
			panic(err)
		}
		files := token.NewFileSet()
		file, err := parser.ParseFile(files, row.File, raw, 0)
		if err != nil {
			panic(err)
		}
		found := false
		for _, declaration := range file.Decls {
			function, ok := declaration.(*ast.FuncDecl)
			if !ok || function.Name.Name != row.Test {
				continue
			}
			found = true
			var cases *ast.CompositeLit
			var end token.Pos
			for _, statement := range function.Body.List {
				if _, ok := statement.(*ast.RangeStmt); ok {
					end = statement.Pos()
					break
				}
				assignment, ok := statement.(*ast.AssignStmt)
				if !ok || len(assignment.Lhs) != 1 || len(assignment.Rhs) != 1 {
					continue
				}
				identifier, ok := assignment.Lhs[0].(*ast.Ident)
				if ok && identifier.Name == row.Variable {
					cases, _ = assignment.Rhs[0].(*ast.CompositeLit)
				}
			}
			if cases == nil || len(cases.Elts) != row.Cases || !end.IsValid() {
				panic(fmt.Sprintf("changed source table: %s", row.Test))
			}
			startOffset := files.Position(function.Body.Lbrace).Offset + 1
			endOffset := files.Position(end).Offset
			row.Prefix = string(raw[startOffset:endOffset])
			row.Line = files.Position(function.Pos()).Line
			for _, element := range cases.Elts {
				row.CaseLines = append(row.CaseLines, files.Position(element.Pos()).Line)
				if row.Variable == "tcs" {
					entry, ok := element.(*ast.KeyValueExpr)
					if !ok {
						panic("missing map key")
					}
					key, ok := entry.Key.(*ast.BasicLit)
					if !ok || key.Kind != token.STRING {
						panic("non-literal map key")
					}
					decoded, err := strconv.Unquote(key.Value)
					if err != nil {
						panic(err)
					}
					row.CaseKeys = append(row.CaseKeys, decoded)
				}
			}
			row.SourceHash = fmt.Sprintf("%x", sha256.Sum256(raw))
			break
		}
		if !found {
			panic("missing source test: " + row.Test)
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(tables); err != nil {
		panic(err)
	}
}
