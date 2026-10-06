// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only Go 1.27.1 float conversion and original input capture; downloads disabled.
package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"math"
	"os"
	"runtime"
	"strconv"
	"strings"
)

type request struct {
	ID     string `json:"id"`
	Input  []byte `json:"input"`
	Origin string `json:"origin,omitempty"`
}

func text(expr ast.Expr) string {
	switch v := expr.(type) {
	case *ast.BasicLit:
		if v.Kind != token.STRING {
			panic("expected string literal")
		}
		s, err := strconv.Unquote(v.Value)
		if err != nil {
			panic(err)
		}
		return s
	case *ast.BinaryExpr:
		if v.Op != token.ADD {
			panic("expected string concatenation")
		}
		return text(v.X) + text(v.Y)
	case *ast.CallExpr:
		selector, ok := v.Fun.(*ast.SelectorExpr)
		if !ok || selector.Sel.Name != "Repeat" || len(v.Args) != 2 {
			panic("unexpected input expression")
		}
		pkg, ok := selector.X.(*ast.Ident)
		if !ok || pkg.Name != "strings" {
			panic("unexpected input constructor")
		}
		n, ok := v.Args[1].(*ast.BasicLit)
		if !ok || n.Kind != token.INT {
			panic("expected repeat count")
		}
		count, err := strconv.Atoi(n.Value)
		if err != nil {
			panic(err)
		}
		return strings.Repeat(text(v.Args[0]), count)
	}
	panic("unexpected original input expression")
}

func original(path string) []request {
	set := token.NewFileSet()
	file, err := parser.ParseFile(set, path, nil, 0)
	if err != nil {
		panic(err)
	}
	var out []request
	ast.Inspect(file, func(node ast.Node) bool {
		v, ok := node.(*ast.ValueSpec)
		if !ok || len(v.Names) != 1 || v.Names[0].Name != "atoftests" {
			return true
		}
		list, ok := v.Values[0].(*ast.CompositeLit)
		if !ok {
			panic("expected original table")
		}
		for _, item := range list.Elts {
			entry, ok := item.(*ast.CompositeLit)
			if !ok || len(entry.Elts) != 3 {
				panic("unexpected original case")
			}
			out = append(out, request{
				ID:     fmt.Sprintf("go-atof-%04d", len(out)),
				Input:  []byte(text(entry.Elts[0])),
				Origin: fmt.Sprintf("src/internal/strconv/atof_test.go:%d", set.Position(entry.Pos()).Line),
			})
		}
		return false
	})
	if len(out) == 0 {
		panic("original table not found")
	}
	return out
}

func main() {
	if len(os.Args) == 3 && os.Args[1] == "--cases" {
		if err := json.NewEncoder(os.Stdout).Encode(original(os.Args[2])); err != nil {
			panic(err)
		}
		return
	}
	if len(os.Args) != 1 {
		panic("unexpected arguments")
	}
	var inputs []request
	if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil {
		panic(err)
	}
	out := make([]map[string]any, 0, len(inputs))
	for _, r := range inputs {
		v, err := strconv.ParseFloat(string(r.Input), 64)
		kind := ""
		if err != nil {
			switch {
			case errors.Is(err, strconv.ErrSyntax):
				kind = "syntax"
			case errors.Is(err, strconv.ErrRange):
				kind = "range"
			default:
				panic(err)
			}
		}
		out = append(out, map[string]any{
			"id": r.ID, "input": r.Input, "origin": r.Origin,
			"value_bits": fmt.Sprintf("%016x", math.Float64bits(v)), "error": kind,
		})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"go_version": runtime.Version(), "architecture": runtime.GOARCH,
		"runtime_commit": "862c888e612ac346c7c4d99c9392bdfd265f33b0",
		"cases":          out,
	}); err != nil {
		panic(err)
	}
}
