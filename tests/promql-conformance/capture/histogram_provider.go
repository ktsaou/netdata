// SPDX-License-Identifier: GPL-3.0-or-later
// Original engine queries qualify replacement of a same-text ordinary storage warning.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"log/slog"
	"os"
	"time"

	"github.com/prometheus/prometheus/model/labels"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"github.com/prometheus/prometheus/promql/parser/posrange"
	"github.com/prometheus/prometheus/storage"
	"github.com/prometheus/prometheus/util/annotations"
	"netdata-promql-conformance-oracle/codec"
	"netdata-promql-conformance-oracle/wire"
)

type source struct {
	wire.Source
	message string
}

func (s source) Querier(start, end int64) (storage.Querier, error) {
	q, err := s.Source.Querier(start, end)
	return querier{q, s.message}, err
}

type querier struct {
	storage.Querier
	message string
}

func (q querier) Select(ctx context.Context, sorted bool, hints *storage.SelectHints, matchers ...*labels.Matcher) storage.SeriesSet {
	return seriesSet{q.Querier.Select(ctx, sorted, hints, matchers...), q.message}
}

type seriesSet struct {
	storage.SeriesSet
	message string
}

func (s seriesSet) Warnings() annotations.Annotations {
	var result annotations.Annotations
	result.Merge(s.SeriesSet.Warnings())
	result.Add(errors.New(s.message))
	return result
}

func main() {
	var dataset wire.Dataset
	var requests []wire.Request
	read := func(path string, value any) {
		file, err := os.Open(path)
		if err != nil {
			panic(err)
		}
		defer file.Close()
		if err = json.NewDecoder(file).Decode(value); err != nil {
			panic(err)
		}
	}
	read(os.Args[1], &dataset)
	read(os.Args[2], &requests)
	results := []wire.Result{}
	messages := map[string]string{}
	for _, request := range requests {
		delayed := request.Profile == "upstream-tests"
		name := ""
		if delayed {
			name = "m"
		}
		message := annotations.NewHistogramQuantileForcedMonotonicityInfo(name, posrange.PositionRange{}, 0, 0, 0, 0).Error()
		messages[request.Profile] = message
		engine := promql.NewEngine(promql.EngineOpts{
			Logger: slog.New(slog.NewTextHandler(io.Discard, nil)), MaxSamples: 10000000, Timeout: time.Minute,
			LookbackDelta: 5 * time.Minute, EnableAtModifier: true, EnableNegativeOffset: true,
			EnableDelayedNameRemoval: delayed, UseStartTimestamps: delayed,
			NoStepSubqueryIntervalFn: func(int64) int64 { return 60000 },
			Parser:                   parser.NewParser(parser.Options{ExperimentalDurationExpr: true}),
		})
		query, err := engine.NewRangeQuery(context.Background(), source{wire.Source{Dataset: dataset}, message},
			promql.NewPrometheusQueryOpts(false, time.Duration(request.Lookback)*time.Millisecond), request.Query,
			time.UnixMilli(request.Start), time.UnixMilli(request.End), time.Duration(request.Step)*time.Millisecond)
		if err != nil {
			panic(err)
		}
		results = append(results, codec.Result(request.ID, request.Query, query.Exec(context.Background())))
		query.Close()
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"schema_version": 1, "commit": wire.ReferenceCommit, "results": results, "provider_messages": messages,
	}); err != nil {
		panic(err)
	}
}
