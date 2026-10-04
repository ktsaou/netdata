// SPDX-License-Identifier: GPL-3.0-or-later
// Compiled only as an additional test file in the verified upstream source facade.
package parser

import (
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"reflect"
	"runtime"
	"testing"
	"unicode"
)

type netdataToken struct {
	Type     int    `json:"type"`
	Position int    `json:"position"`
	Value    []byte `json:"value"`
}

func netdataTokens(items []Item) []netdataToken {
	out := make([]netdataToken, 0, len(items))
	for _, item := range items {
		out = append(out, netdataToken{int(item.Typ), int(item.Pos), []byte(item.Val)})
	}
	return out
}

type netdataTree struct {
	nodes []map[string]any
}

// Node references keep deeply nested ASTs below the transport's JSON depth limit.
func (tree *netdataTree) add(expr Expr) any {
	if expr == nil {
		return nil
	}
	value := reflect.ValueOf(expr)
	if value.IsNil() {
		return nil
	}
	index := len(tree.nodes)
	tree.nodes = append(tree.nodes, nil)
	tree.nodes[index] = map[string]any{"node": value.Elem().Type().Name(), "fields": tree.value(value.Elem())}
	return map[string]any{"node_ref": index}
}

// Encode Go strings as bytes and floats as bits, including inside nested AST fields.
func (tree *netdataTree) value(value reflect.Value) any {
	if !value.IsValid() {
		return nil
	}
	if value.Kind() == reflect.Interface {
		if value.IsNil() {
			return nil
		}
		return tree.value(value.Elem())
	}
	if value.Kind() == reflect.Pointer {
		if value.IsNil() {
			return nil
		}
		if expr, ok := value.Interface().(Expr); ok {
			return tree.add(expr)
		}
		return tree.value(value.Elem())
	}
	switch value.Kind() {
	case reflect.String:
		return []byte(value.String())
	case reflect.Float32, reflect.Float64:
		return map[string]any{"float_bits": fmt.Sprintf("%016x", math.Float64bits(value.Float()))}
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return value.Int()
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return value.Uint()
	case reflect.Bool:
		return value.Bool()
	case reflect.Slice, reflect.Array:
		if value.Kind() == reflect.Slice && value.IsNil() {
			return nil
		}
		out := make([]any, value.Len())
		for i := range out {
			out[i] = tree.value(value.Index(i))
		}
		return out
	case reflect.Struct:
		out := map[string]any{}
		for i := 0; i < value.NumField(); i++ {
			field := value.Type().Field(i)
			if field.PkgPath == "" {
				out[field.Name] = tree.value(value.Field(i))
			}
		}
		return out
	default:
		panic(fmt.Sprintf("unsupported capture field %s", value.Type()))
	}
}

func netdataAST(expr Expr) any {
	tree := &netdataTree{nodes: []map[string]any{}}
	root := tree.add(expr)
	if root == nil {
		return nil
	}
	return map[string]any{"root": root, "nodes": tree.nodes}
}

func netdataErrors(err error, input string) []map[string]any {
	out := []map[string]any{}
	if err == nil {
		return out
	}
	var list ParseErrors
	if !errors.As(err, &list) {
		panic(fmt.Sprintf("unexpected parser error %T", err))
	}
	for _, e := range list {
		entry := map[string]any{"start": int(e.PositionRange.Start),
			"end": int(e.PositionRange.End), "line_offset": e.LineOffset,
			"message": []byte(e.Err.Error())}
		if e.Query == input {
			entry["query_ref"] = "input"
		} else {
			entry["query"] = []byte(e.Query)
		}
		out = append(out, entry)
	}
	return out
}

func TestNetdataCaptureFrontend(t *testing.T) {
	path := os.Getenv("NETDATA_PROMQL_CAPTURE_FRONTEND")
	if path == "" {
		t.Fatal("capture output path is required")
	}
	lexers := []map[string]any{}
	for groupIndex, group := range tests {
		for index, test := range group.tests {
			lexer := &Lexer{input: test.input, seriesDesc: test.seriesDesc}
			items := []Item{}
			// ERROR can be recoverable; preserve the same complete stream as TestLexer.
			for lexer.state = lexStatements; lexer.state != nil; {
				items = append(items, Item{})
				lexer.NextItem(&items[len(items)-1])
			}
			lexers = append(lexers, map[string]any{
				"id":    fmt.Sprintf("lexer:%02d:%03d", groupIndex, index),
				"group": group.name, "input": []byte(test.input),
				"series_description": test.seriesDesc, "declared_failure": test.fail,
				"declared_tokens": netdataTokens(test.expected), "tokens": netdataTokens(items),
			})
		}
	}
	if len(tests) != 17 || len(lexers) != 157 {
		t.Fatalf("lexer tables changed: %d groups, %d cases", len(tests), len(lexers))
	}
	options := Options{EnableExperimentalFunctions: true, ExperimentalDurationExpr: true, EnableExtendedRangeSelectors: true}
	parser := NewParser(options)
	expressions := []map[string]any{}
	for i, test := range testExpr {
		expr, err := parser.ParseExpr(test.input)
		expressions = append(expressions, map[string]any{
			"id": fmt.Sprintf("parser:%03d", i), "input": []byte(test.input),
			"declared_failure": test.fail, "declared_ast": netdataAST(test.expected),
			"ast": netdataAST(expr), "errors": netdataErrors(err, test.input),
			"declared_errors": netdataErrors(test.errors, test.input),
		})
	}
	if len(expressions) != 356 {
		t.Fatalf("expression table changed: %d", len(expressions))
	}
	// Capture before the original assertions clear actual matchers for deep comparison.
	t.Run("upstream-lexer-assertions", TestLexer)
	t.Run("upstream-parser-assertions", TestParseExpressions)
	if t.Failed() {
		return
	}
	output := map[string]any{"schema_version": 1, "reference": "prometheus/prometheus",
		"commit":     "5241a27fe3c6983549fccc32f6e65917408c63cd",
		"go_version": runtime.Version(), "go_os": runtime.GOOS, "go_arch": runtime.GOARCH,
		"unicode_version": unicode.Version, "parser_options": options,
		"lexers": lexers, "expressions": expressions, "original_assertions_passed": true}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		t.Fatal(err)
	}
	encoder := json.NewEncoder(file)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(output); err != nil {
		file.Close()
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
}
