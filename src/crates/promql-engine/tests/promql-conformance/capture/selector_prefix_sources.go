// SPDX-License-Identifier: GPL-3.0-or-later
// Extract the literal table actually iterated by the pinned original test.
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

func main() {
	if len(os.Args) != 2 || runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		panic("expected pinned Go toolchain and Prometheus source directory")
	}
	const source = "model/labels/regexp_test.go"
	const name = "TestStringMatcherFromRegexp_LiteralPrefix"
	raw, err := os.ReadFile(filepath.Join(os.Args[1], source))
	if err != nil {
		panic(err)
	}
	positions := token.NewFileSet()
	file, err := parser.ParseFile(positions, source, raw, 0)
	if err != nil {
		panic(err)
	}
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
	if !ok || len(table.Elts) != 5 {
		panic("original prefix table changed")
	}
	cases := make([]map[string]any, 0, len(table.Elts))
	positive, negative := 0, 0
	for _, expression := range table.Elts {
		entry, ok := expression.(*ast.CompositeLit)
		if !ok || len(entry.Elts) != 4 {
			panic("original prefix fields changed")
		}
		row := map[string]any{}
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
			case "expectedLiteralPrefixMatchers":
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
				if key.Name == "expectedMatches" {
					positive += len(values)
				} else {
					negative += len(values)
				}
			default:
				panic("unexpected original field")
			}
		}
		cases = append(cases, row)
	}
	if positive != 32 || negative != 20 {
		panic("original prefix declaration roster changed")
	}
	start, end := positions.Position(function.Pos()), positions.Position(function.End())
	result := map[string]any{
		"schema_version": 1, "commit": "5241a27fe3c6983549fccc32f6e65917408c63cd",
		"source": source, "source_sha256": digest(raw), "function": name,
		"function_line": start.Line, "function_sha256": digest(raw[start.Offset:end.Offset]),
		"cases": cases,
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
