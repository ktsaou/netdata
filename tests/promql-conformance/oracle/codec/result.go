// SPDX-License-Identifier: GPL-3.0-or-later
package codec

import (
	"errors"
	"fmt"
	"sort"

	"github.com/prometheus/prometheus/promql"
	"github.com/prometheus/prometheus/promql/parser"
	"netdata-promql-conformance-oracle/wire"
)

func Error(phase string, err error) *wire.Failure {
	out := &wire.Failure{Phase: phase, Type: fmt.Sprintf("%T", err), Message: []byte(err.Error())}
	var parsed parser.ParseErrors
	if errors.As(err, &parsed) {
		for _, e := range parsed {
			out.ParseErrors = append(out.ParseErrors, wire.ParseFailure{int(e.PositionRange.Start), int(e.PositionRange.End), e.LineOffset, []byte(e.Err.Error())})
		}
	}
	return out
}
func Result(id, query string, v *promql.Result) wire.Result {
	warnings, infos := v.Warnings.AsStrings(query, 0, 0)
	out := wire.Result{ID: id, Rows: []wire.Series{}, Warnings: wire.ByteMessages(warnings), Infos: wire.ByteMessages(infos)}
	if v.Err != nil {
		out.Kind = "error"
		out.Error = Error("execution", v.Err)
		return out
	}
	switch x := v.Value.(type) {
	case promql.Scalar:
		out.Kind = "scalar"
		out.Rows = append(out.Rows, wire.Series{wire.Labelset{}, []wire.Point{wire.FloatPoint(x.T, x.V)}})
	case promql.String:
		out.Kind = "string"
		out.Rows = append(out.Rows, wire.Series{wire.Labelset{}, []wire.Point{{Time: x.T, Kind: "string", String: []byte(x.V)}}})
	case promql.Vector:
		out.Kind = "vector"
		for _, s := range x {
			p := wire.FloatPoint(s.T, s.F)
			if s.H != nil {
				p = wire.Point{Time: s.T, Kind: "float_histogram", FloatHistogram: wire.EncodeFloatHistogram(s.H)}
			}
			out.Rows = append(out.Rows, wire.Series{wire.EncodeLabels(s.Metric), []wire.Point{p}})
		}
	case promql.Matrix:
		out.Kind = "matrix"
		for _, s := range x {
			points := []wire.Point{}
			for _, p := range s.Floats {
				points = append(points, wire.FloatPoint(p.T, p.F))
			}
			for _, p := range s.Histograms {
				points = append(points, wire.Point{Time: p.T, Kind: "float_histogram", FloatHistogram: wire.EncodeFloatHistogram(p.H)})
			}
			sort.Slice(points, func(i, j int) bool { return points[i].Time < points[j].Time })
			out.Rows = append(out.Rows, wire.Series{wire.EncodeLabels(s.Metric), points})
		}
	default:
		panic(fmt.Sprintf("unhandled upstream value %T", x))
	}
	return out
}
