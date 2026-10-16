// SPDX-License-Identifier: GPL-3.0-or-later
// Extract literal tables actually iterated by the pinned original tests.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"unicode"
)

func literalString(expression ast.Expr) []byte {
	literal, ok := expression.(*ast.BasicLit)
	if !ok || literal.Kind != token.STRING {
		panic("expected a literal Go string")
	}
	value, err := strconv.Unquote(literal.Value)
	if err != nil {
		panic(err)
	}
	return []byte(value)
}

func literalStrings(expression ast.Expr) [][]byte {
	literal, ok := expression.(*ast.CompositeLit)
	if !ok {
		panic("expected a literal string slice")
	}
	array, ok := literal.Type.(*ast.ArrayType)
	if !ok || array.Len != nil {
		panic("expected an unsized string slice")
	}
	element, ok := array.Elt.(*ast.Ident)
	if !ok || element.Name != "string" {
		panic("expected string slice elements")
	}
	result := make([][]byte, 0, len(literal.Elts))
	for _, value := range literal.Elts {
		result = append(result, literalString(value))
	}
	return result
}

func digest(value []byte) string {
	result := sha256.Sum256(value)
	return hex.EncodeToString(result[:])
}

const source = "model/labels/regexp_test.go"

type tableSpec struct {
	name, countField            string
	entries, positive, negative int
	locations                   bool
}

func literalPositions(expression ast.Expr, positions *token.FileSet) []map[string]int {
	values := []ast.Expr{expression}
	if list, ok := expression.(*ast.CompositeLit); ok {
		values = list.Elts
	}
	result := make([]map[string]int, 0, len(values))
	for _, value := range values {
		start, end := positions.Position(value.Pos()), positions.Position(value.End())
		result = append(result, map[string]int{
			"line": start.Line, "column": start.Column, "offset": start.Offset, "end_offset": end.Offset,
		})
	}
	return result
}

func extract(raw []byte, file *ast.File, positions *token.FileSet, spec tableSpec) map[string]any {
	name := spec.name
	var function *ast.FuncDecl
	for _, declaration := range file.Decls {
		if candidate, ok := declaration.(*ast.FuncDecl); ok && candidate.Name.Name == name {
			if function != nil {
				panic("duplicate original test")
			}
			function = candidate
		}
	}
	if function == nil || len(function.Body.List) != 1 {
		panic("original test body changed")
	}
	iteration, ok := function.Body.List[0].(*ast.RangeStmt)
	if !ok {
		panic("original test no longer iterates one literal table")
	}
	table, ok := iteration.X.(*ast.CompositeLit)
	if !ok || len(table.Elts) != spec.entries {
		panic("original literal table changed")
	}
	cases := make([]map[string]any, 0, len(table.Elts))
	positive, negative := 0, 0
	for _, expression := range table.Elts {
		entry, ok := expression.(*ast.CompositeLit)
		if !ok || len(entry.Elts) != 4 {
			panic("original literal fields changed")
		}
		row := map[string]any{}
		locations := map[string][]map[string]int{}
		for _, expression := range entry.Elts {
			field, ok := expression.(*ast.KeyValueExpr)
			if !ok {
				panic("expected named original fields")
			}
			key, ok := field.Key.(*ast.Ident)
			if !ok {
				panic("expected original field identifier")
			}
			if _, duplicate := row[key.Name]; duplicate {
				panic("duplicate original field")
			}
			switch key.Name {
			case "pattern":
				row[key.Name] = literalString(field.Value)
				row["line"] = positions.Position(field.Pos()).Line
				locations[key.Name] = literalPositions(field.Value, positions)
			case spec.countField:
				literal, ok := field.Value.(*ast.BasicLit)
				if !ok || literal.Kind != token.INT {
					panic("expected original literal matcher count")
				}
				count, err := strconv.Atoi(literal.Value)
				if err != nil {
					panic(err)
				}
				row[key.Name] = count
			case "expectedMatches", "expectedNotMatches":
				values := literalStrings(field.Value)
				row[key.Name] = values
				locations[key.Name] = literalPositions(field.Value, positions)
				if key.Name == "expectedMatches" {
					positive += len(values)
				} else {
					negative += len(values)
				}
			default:
				panic("unexpected original field")
			}
		}
		if spec.locations {
			row["declaration_positions"] = locations
		}
		cases = append(cases, row)
	}
	if positive != spec.positive || negative != spec.negative {
		panic("original literal declaration roster changed")
	}
	start, end := positions.Position(function.Pos()), positions.Position(function.End())
	return map[string]any{
		"schema_version": 1, "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
		"source": source, "source_sha256": digest(raw), "function": name,
		"function_line": start.Line, "function_sha256": digest(raw[start.Offset:end.Offset]),
		"cases": cases,
	}
}

func main() {
	if len(os.Args) < 2 || runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		panic("expected pinned Go toolchain and Prometheus source directory")
	}
	specs := map[string]tableSpec{
		"TestStringMatcherFromRegexp_LiteralPrefix": {"TestStringMatcherFromRegexp_LiteralPrefix", "expectedLiteralPrefixMatchers", 5, 32, 20, false},
		"TestStringMatcherFromRegexp_LiteralSuffix": {"TestStringMatcherFromRegexp_LiteralSuffix", "expectedLiteralSuffixMatchers", 4, 32, 24, true},
		"TestStringMatcherFromRegexp_Quest":         {"TestStringMatcherFromRegexp_Quest", "expectedZeroOrOneMatchers", 6, 28, 17, true},
	}
	raw, err := os.ReadFile(filepath.Join(os.Args[1], source))
	if err != nil {
		panic(err)
	}
	positions := token.NewFileSet()
	file, err := parser.ParseFile(positions, source, raw, 0)
	if err != nil {
		panic(err)
	}
	var result any
	if len(os.Args) == 2 {
		result = extract(raw, file, positions, specs["TestStringMatcherFromRegexp_LiteralPrefix"])
	} else {
		tables := make([]map[string]any, 0, len(os.Args)-2)
		seen := map[string]bool{}
		for _, name := range os.Args[2:] {
			spec, ok := specs[name]
			if !ok || seen[name] {
				panic("unexpected or duplicate original table name")
			}
			seen[name] = true
			tables = append(tables, extract(raw, file, positions, spec))
		}
		result = map[string]any{
			"schema_version": 1, "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
			"source": source, "source_sha256": digest(raw), "tables": tables,
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
