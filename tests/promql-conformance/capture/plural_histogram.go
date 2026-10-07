// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise independent name-removal options without changing the pinned engine.
package main

import (
	"context"
	"encoding/json"
	"io"
	"log/slog"
	"os"
	"runtime"
	"time"
	"unicode"

	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"netdata-promql-conformance-oracle/codec"
	"netdata-promql-conformance-oracle/wire"
)

type request struct {
	wire.Request
	Delayed *bool `json:"delayed_name_removal,omitempty"`
}

func evaluate(data wire.Dataset, r request) wire.Result {
	out := wire.Result{ID: r.ID, Kind: "error", Rows: []wire.Series{}, Warnings: [][]byte{}, Infos: [][]byte{}}
	text, err := r.Text()
	if err != nil {
		out.Error = codec.Error("request", err)
		return out
	}
	extended := r.Profile == "upstream-tests"
	delayed := extended
	if r.Delayed != nil {
		delayed = *r.Delayed
	}
	engine := promql.NewEngine(promql.EngineOpts{
		Logger: slog.New(slog.NewTextHandler(io.Discard, nil)), MaxSamples: 10000000, Timeout: time.Minute,
		LookbackDelta: 5 * time.Minute, EnableAtModifier: true, EnableNegativeOffset: true,
		EnableDelayedNameRemoval: delayed, UseStartTimestamps: extended,
		NoStepSubqueryIntervalFn: func(int64) int64 { return 60000 },
		Parser: parser.NewParser(parser.Options{ExperimentalDurationExpr: true, EnableExperimentalFunctions: extended,
			EnableExtendedRangeSelectors: extended, EnableBinopFillModifiers: extended}),
	})
	opts := promql.NewPrometheusQueryOpts(false, 5*time.Minute)
	var query promql.Query
	if r.Mode == "instant" {
		query, err = engine.NewInstantQuery(context.Background(), wire.Source{Dataset: data}, opts, text, time.UnixMilli(r.Time))
	} else {
		query, err = engine.NewRangeQuery(context.Background(), wire.Source{Dataset: data}, opts, text,
			time.UnixMilli(r.Start), time.UnixMilli(r.End), time.Duration(r.Step)*time.Millisecond)
	}
	if err != nil {
		out.Error = codec.Error("construction", err)
		return out
	}
	defer query.Close()
	return codec.Result(r.ID, text, query.Exec(context.Background()))
}

func main() {
	read := func(path string, destination any) {
		file, err := os.Open(path)
		if err != nil {
			panic(err)
		}
		defer file.Close()
		decoder := json.NewDecoder(file)
		decoder.DisallowUnknownFields()
		if err = decoder.Decode(destination); err != nil {
			panic(err)
		}
		var trailing any
		if err = decoder.Decode(&trailing); err != io.EOF {
			panic("trailing capture input")
		}
	}
	var data wire.Dataset
	var requests []request
	read(os.Args[1], &data)
	read(os.Args[2], &requests)
	results := make([]wire.Result, 0, len(requests))
	for _, r := range requests {
		results = append(results, evaluate(data, r))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"schema_version": wire.SchemaVersion, "reference": wire.ReferenceModule, "commit": wire.ReferenceCommit,
		"go_version": runtime.Version(), "go_arch": runtime.GOARCH, "go_os": runtime.GOOS,
		"unicode_version": unicode.Version, "results": results,
	}); err != nil {
		panic(err)
	}
}
