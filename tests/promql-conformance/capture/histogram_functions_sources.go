// SPDX-License-Identifier: GPL-3.0-or-later
// Bind exact helper owners and untouched source assertions before observation.
package main

import (
	"encoding/hex"
	"encoding/json"
	promparser "github.com/prometheus/prometheus/promql/parser"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"regexp"
	"strings"
)

func main() {
	if len(os.Args) != 2 {
		panic("expected pinned Prometheus source root")
	}
	definitions := map[string]any{}
	for name, source := range map[string]string{
		"TestTrimBuckets_HistogramFractionCrossCheck": "promql/quantile_test.go",
		"HistogramFraction":                           "promql/quantile.go", "HistogramQuantile": "promql/quantile.go",
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
				panic("duplicate histogram helper definition")
			}
			lines := []int{}
			ast.Inspect(function, func(node ast.Node) bool {
				if call, ok := node.(*ast.CallExpr); ok {
					if ident, ok := call.Fun.(*ast.Ident); ok && ident.Name == "HistogramFraction" {
						lines = append(lines, files.Position(call.Pos()).Line)
					}
				}
				return true
			})
			start, end := files.Position(function.Pos()), files.Position(function.End())
			definitions[name] = map[string]any{"source": source, "line": start.Line,
				"body": string(raw[start.Offset:end.Offset]), "call_lines": lines}
		}
	}
	if len(definitions) != 3 {
		panic("changed histogram helper source definitions")
	}
	raw, err := os.ReadFile(filepath.Join(os.Args[1], "promql/promqltest/testdata/native_histograms.test"))
	if err != nil {
		panic(err)
	}
	lines := strings.Split(string(raw), "\n")
	header := regexp.MustCompile(`^[ \t]*eval(?:_[a-z]+)?[ \t]+instant(?:[ \t]+at[ \t]+[^ \t]+)?[ \t]+(.+)$`)
	commands := []any{}
	for index, line := range lines {
		match := header.FindStringSubmatch(line)
		if match == nil {
			continue
		}
		expression, err := promparser.NewParser(promparser.Options{
			EnableExperimentalFunctions: true, ExperimentalDurationExpr: true,
			EnableExtendedRangeSelectors: true, EnableBinopFillModifiers: true}).ParseExpr(match[1])
		if err != nil {
			panic(err)
		}
		call, ok := expression.(*promparser.Call)
		if !ok || (call.Func.Name != "histogram_fraction" && call.Func.Name != "histogram_quantile") {
			continue
		}
		number := index + 1
		if call.Func.Name == "histogram_quantile" && (number == 1904 || number == 1908 || number == 1911 || number == 1914) {
			continue
		}
		arguments := []any{}
		for _, arg := range call.Args {
			pos := arg.PositionRange()
			arguments = append(arguments, map[string]any{"start": pos.Start, "end": pos.End})
		}
		rows := []int{}
		for next := index + 1; next < len(lines) && strings.TrimSpace(lines[next]) != ""; next++ {
			if strings.HasPrefix(strings.TrimSpace(lines[next]), "{") {
				rows = append(rows, next+1)
			}
		}
		commands = append(commands, map[string]any{"line": number, "query": hex.EncodeToString([]byte(match[1])),
			"function": call.Func.Name, "argument_ranges": arguments, "row_lines": rows})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"definitions": definitions, "commands": commands}); err != nil {
		panic(err)
	}
}
