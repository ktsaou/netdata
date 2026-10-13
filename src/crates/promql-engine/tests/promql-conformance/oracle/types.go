// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"github.com/prometheus/prometheus/model/histogram"
	"github.com/prometheus/prometheus/model/labels"
	"netdata-promql-conformance-oracle/wire"
)

const schemaVersion = wire.SchemaVersion
const referenceCommit = wire.ReferenceCommit
const referenceModule = wire.ReferenceModule

type binary64 = wire.Binary64
type label = wire.Label
type labelset = wire.Labelset
type floatHistogram = wire.FloatHistogram
type integerHistogram = wire.IntegerHistogram
type point = wire.Point
type series = wire.Series
type dataset = wire.Dataset
type request = wire.Request
type parseFailure = wire.ParseFailure
type failure = wire.Failure
type result = wire.Result

func number(f float64) binary64              { return wire.Number(f) }
func encodeLabels(ls labels.Labels) labelset { return wire.EncodeLabels(ls) }
func encodeFloatHistogram(h *histogram.FloatHistogram) *floatHistogram {
	return wire.EncodeFloatHistogram(h)
}
func encodeIntegerHistogram(h *histogram.Histogram) *integerHistogram {
	return wire.EncodeIntegerHistogram(h)
}
func floatPoint(t int64, f float64) point { return wire.FloatPoint(t, f) }
func byteMessages(ss []string) [][]byte   { return wire.ByteMessages(ss) }
