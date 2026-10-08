// SPDX-License-Identifier: GPL-3.0-or-later
// Observe unchanged pinned Prometheus arithmetic; see the conformance guide.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"io"
	"log/slog"
	"netdata-promql-conformance-oracle/codec"
	"netdata-promql-conformance-oracle/wire"
	"os"
	"strconv"
	"time"
)

type Case struct {
	ID    string       `json:"id"`
	Owner string       `json:"owner"`
	Query string       `json:"query"`
	Data  wire.Dataset `json:"data"`
}

func main() {
	useStart, e := strconv.ParseBool(os.Args[2])
	if e != nil {
		panic(e)
	}
	raw, e := os.ReadFile(os.Args[1])
	if e != nil {
		panic(e)
	}
	var cases []Case
	if e = json.Unmarshal(raw, &cases); e != nil {
		panic(e)
	}
	engine := promql.NewEngine(promql.EngineOpts{Logger: slog.New(slog.NewTextHandler(io.Discard, nil)), MaxSamples: 100000, Timeout: 2 * time.Second, LookbackDelta: 5 * time.Minute, EnableAtModifier: true, EnableNegativeOffset: true, EnableDelayedNameRemoval: true, UseStartTimestamps: useStart, NoStepSubqueryIntervalFn: func(int64) int64 { return 60000 }, Parser: parser.NewParser(parser.Options{ExperimentalDurationExpr: true, EnableExperimentalFunctions: true, EnableExtendedRangeSelectors: true, EnableBinopFillModifiers: true})})
	enc := json.NewEncoder(os.Stdout)
	for _, c := range cases {
		func() {
			row := map[string]any{"id": c.ID, "owner": c.Owner, "request": c}
			defer func() {
				if x := recover(); x != nil {
					row["panic"] = fmt.Sprint(x)
				}
				if e := enc.Encode(row); e != nil {
					panic(e)
				}
			}()
			q, e := engine.NewInstantQuery(context.Background(), wire.Source{Dataset: c.Data}, promql.NewPrometheusQueryOpts(false, 300000*time.Millisecond), c.Query, time.UnixMilli(0))
			if e != nil {
				row["error"] = e.Error()
				return
			}
			defer q.Close()
			row["actual"] = codec.Result(c.ID, c.Query, q.Exec(context.Background()))
		}()
	}
}
