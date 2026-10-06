// SPDX-License-Identifier: GPL-3.0-or-later
// Reference outputs come only from the pinned runtime and Prometheus label helper.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"runtime"
	"strconv"

	"github.com/prometheus/prometheus/model/labels"
)

type request struct {
	ID     string `json:"id"`
	Bits   string `json:"bits"`
	Mode   string `json:"mode"`
	Origin string `json:"origin"`
}

type result struct {
	request
	Text           string `json:"text"`
	RoundtripBits  string `json:"roundtrip_bits"`
	RoundtripError string `json:"roundtrip_error"`
}

func original() {
	type originalRow struct {
		Index      int    `json:"index"`
		Line       int    `json:"line"`
		Expression string `json:"expression"`
		Bits       string `json:"bits"`
		Format     string `json:"format"`
		Precision  int    `json:"precision"`
		Declared   string `json:"declared"`
		Text       string `json:"text"`
		AppendText string `json:"append_text"`
	}
	rows := make([]originalRow, 0, len(ftoatests))
	selected := []request{}
	for i, row := range ftoatests {
		text := strconv.FormatFloat(row.f, row.fmt, row.prec, 64)
		appended := string(strconv.AppendFloat([]byte("abc"), row.f, row.fmt, row.prec, 64))
		if text != row.s || appended != "abc"+row.s {
			panic(fmt.Sprintf("original assertion differs: row %d", i))
		}
		bits := fmt.Sprintf("%016x", math.Float64bits(row.f))
		rows = append(rows, originalRow{i, originalLines[i], originalExpressions[i], bits,
			string(row.fmt), row.prec, row.s, text, appended})
		if row.fmt == 'g' && (row.prec == -1 || row.prec == 2) {
			mode := "shortest"
			if row.prec == 2 {
				mode = "two"
			}
			selected = append(selected, request{fmt.Sprintf("original-go-%03d", i), bits, mode,
				fmt.Sprintf("ftoa_test.go:%d:%d", originalLines[i], i)})
		}
	}
	if len(rows) != 148 || len(selected) != 38 {
		panic("original formatting identities changed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"rows": rows, "selected": selected}); err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) == 2 && os.Args[1] == "--cases" {
		original()
		return
	}
	var requests []request
	decoder := json.NewDecoder(os.Stdin)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]result, 0, len(requests))
	for _, req := range requests {
		bits, err := strconv.ParseUint(req.Bits, 16, 64)
		if err != nil || len(req.Bits) != 16 {
			panic("invalid binary64 input")
		}
		value := math.Float64frombits(bits)
		var text string
		switch req.Mode {
		case "shortest", "two":
			precision, fmtText := -1, "%g"
			if req.Mode == "two" {
				precision, fmtText = 2, "%.2g"
			}
			text = strconv.FormatFloat(value, 'g', precision, 64)
			if text != fmt.Sprintf(fmtText, value) || "abc"+text != string(strconv.AppendFloat([]byte("abc"), value, 'g', precision, 64)) {
				panic("original runtime formatting owners disagree")
			}
		case "openmetrics":
			text = labels.FormatOpenMetricsFloat(value)
		default:
			panic("invalid float formatting mode")
		}
		roundtrip, err := strconv.ParseFloat(text, 64)
		reason := ""
		if err != nil {
			switch err.(*strconv.NumError).Err {
			case strconv.ErrRange:
				reason = "range"
			case strconv.ErrSyntax:
				reason = "syntax"
			default:
				panic(err)
			}
		}
		if req.Mode != "two" && !math.IsNaN(value) {
			expected := bits
			if req.Mode == "openmetrics" && value == 0 {
				expected = 0
			}
			if err != nil || math.Float64bits(roundtrip) != expected {
				panic("original shortest form does not round trip")
			}
		}
		results = append(results, result{req, text, fmt.Sprintf("%016x", math.Float64bits(roundtrip)), reason})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"go_version": runtime.Version(), "architecture": runtime.GOARCH,
		"runtime_commit":    "862c888e612ac346c7c4d99c9392bdfd265f33b0",
		"prometheus_commit": "5241a27fe3c6983549fccc32f6e65917408c63cd", "cases": results,
	}); err != nil {
		panic(err)
	}
}
