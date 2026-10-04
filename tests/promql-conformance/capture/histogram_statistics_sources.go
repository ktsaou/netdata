// SPDX-License-Identifier: GPL-3.0-or-later
// Extract exact statistic owners and root DSL assertions independently of observation hooks.
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
	owners := map[string]string{"simpleHistogramFunc": "promql/functions.go", "funcHistogramCount": "promql/functions.go",
		"funcHistogramSum": "promql/functions.go", "funcHistogramAvg": "promql/functions.go", "histogramVariance": "promql/functions.go",
		"funcHistogramStdDev": "promql/functions.go", "funcHistogramStdVar": "promql/functions.go", "Inc": "util/kahansum/kahansum.go"}
	for name, source := range owners {
		raw, err := os.ReadFile(filepath.Join(os.Args[1], source))
		if err != nil {
			panic(err)
		}
		positions := token.NewFileSet()
		file, err := parser.ParseFile(positions, source, raw, 0)
		if err != nil {
			panic(err)
		}
		for _, node := range file.Decls {
			function, ok := node.(*ast.FuncDecl)
			if !ok || function.Name.Name != name {
				continue
			}
			if _, exists := definitions[name]; exists {
				panic("duplicate statistic source owner")
			}
			start, end := positions.Position(function.Pos()), positions.Position(function.End())
			definitions[name] = map[string]any{"source": source, "line": start.Line, "body": string(raw[start.Offset:end.Offset])}
		}
	}
	if len(definitions) != len(owners) {
		panic("missing statistic source owner")
	}
	functions := map[string]bool{"histogram_count": true, "histogram_sum": true, "histogram_avg": true,
		"histogram_stdvar": true, "histogram_stddev": true}
	header := regexp.MustCompile(`^[ \t]*eval(?:_[a-z]+)?[ \t]+instant(?:[ \t]+at[ \t]+[^ \t]+)?[ \t]+(.+)$`)
	rangeHeader := regexp.MustCompile(`^[ \t]*eval(?:_[a-z]+)?[ \t]+range[ \t]+from[ \t]+(.+)[ \t]+to[ \t]+(.+)[ \t]+step[ \t]+(.+?)[ \t]+(.+)$`)
	commands := []any{}
	for _, name := range []string{"extended_vectors.test", "histograms.test", "native_histograms.test"} {
		source := "promql/promqltest/testdata/" + name
		raw, err := os.ReadFile(filepath.Join(os.Args[1], source))
		if err != nil {
			panic(err)
		}
		lines := strings.Split(string(raw), "\n")
		for index, line := range lines {
			match := header.FindStringSubmatch(line)
			mode, query := "instant", ""
			if match == nil {
				match = rangeHeader.FindStringSubmatch(line)
				if match == nil {
					continue
				}
				mode, query = "range", match[4]
			} else {
				query = match[1]
			}
			expression, err := promparser.NewParser(promparser.Options{EnableExperimentalFunctions: true, ExperimentalDurationExpr: true,
				EnableExtendedRangeSelectors: true, EnableBinopFillModifiers: true}).ParseExpr(query)
			if err != nil {
				panic(err)
			}
			call, ok := expression.(*promparser.Call)
			if !ok || !functions[call.Func.Name] {
				continue
			}
			if len(call.Args) != 1 {
				panic("changed statistic signature")
			}
			pos := call.Args[0].PositionRange()
			rows := []int{}
			for next := index + 1; next < len(lines) && strings.TrimSpace(lines[next]) != ""; next++ {
				if strings.HasPrefix(strings.TrimSpace(lines[next]), "{") {
					rows = append(rows, next+1)
				}
			}
			commands = append(commands, map[string]any{"file": source, "line": index + 1, "query": hex.EncodeToString([]byte(query)), "mode": mode,
				"function": call.Func.Name, "argument_range": map[string]any{"start": pos.Start, "end": pos.End}, "row_lines": rows})
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"definitions": definitions, "commands": commands}); err != nil {
		panic(err)
	}
}
