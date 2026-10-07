// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only control over the original parameter, body, aggregation and cleanup owners.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"time"
	"unicode"

	"github.com/prometheus/prometheus/promql"
	"netdata-promql-conformance-oracle/codec"
	"netdata-promql-conformance-oracle/wire"
)

type quantilePolicyCase struct {
	Request request                    `json:"request"`
	Dataset dataset                    `json:"dataset"`
	Faults  map[string]json.RawMessage `json:"faults"`
}

func quantilePolicyMain() error {
	if len(os.Args) != 2 {
		return fmt.Errorf("policy control requires an input file")
	}
	stream, err := os.Open(os.Args[1])
	if err != nil {
		return err
	}
	defer stream.Close()
	decoder := json.NewDecoder(stream)
	decoder.DisallowUnknownFields()
	var inputs []quantilePolicyCase
	if err := decoder.Decode(&inputs); err != nil {
		return err
	}
	rows := []any{}
	for _, input := range inputs {
		if len(input.Faults) != 0 {
			return fmt.Errorf("policy requires a fault-free child")
		}
		if err := input.Dataset.Validate(); err != nil {
			return err
		}
		r := input.Request
		text, err := r.Text()
		if err != nil {
			return err
		}
		if r.Mode != "range" || r.Profile != "upstream-tests" || r.Lookback <= 0 {
			return fmt.Errorf("unsupported declared policy scope")
		}
		engine, err := newEngine(r.Profile, r.SubqueryStep)
		if err != nil {
			return err
		}
		ctx := context.Background()
		query, err := engine.NewRangeQuery(ctx, wire.Source{Dataset: input.Dataset},
			promql.NewPrometheusQueryOpts(false, time.Duration(r.Lookback)*time.Millisecond), text,
			time.UnixMilli(r.Start), time.UnixMilli(r.End), time.Duration(r.Step)*time.Millisecond)
		if err != nil {
			return err
		}
		value, child, notes, evaluationErr := promql.NetdataQuantilePolicyFixture(ctx, query)
		result := codec.Result(r.ID, text, &promql.Result{Value: value, Warnings: notes, Err: evaluationErr})
		flags := []bool{}
		for _, row := range child {
			flags = append(flags, row.DropName)
		}
		observed := codec.Result("child:"+r.ID, text, &promql.Result{Value: child})
		rows = append(rows, map[string]any{"request": r, "result": result, "child": observed, "child_drop_names": flags})
		query.Close()
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{
		"schema_version": wire.SchemaVersion, "commit": wire.ReferenceCommit, "reference": wire.ReferenceModule,
		"go_version": runtime.Version(), "go_os": runtime.GOOS, "go_arch": runtime.GOARCH,
		"unicode_version": unicode.Version, "cases": rows,
	})
}

func main() {
	if err := quantilePolicyMain(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
