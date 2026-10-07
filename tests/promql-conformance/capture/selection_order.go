// SPDX-License-Identifier: GPL-3.0-or-later
// Observe original Go sorting/heap code and unchanged pinned Prometheus Less methods.
package main

import (
	"container/heap"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"sort"
	"strconv"
)

type Sample struct {
	F       float64
	Ordinal int
}
type Vector []Sample
type request struct {
	ID          string   `json:"id"`
	Values      []string `json:"values"`
	Action      string   `json:"action"`
	Bottom      bool     `json:"bottom"`
	Reverse     bool     `json:"reverse"`
	Pivot       int      `json:"pivot"`
	Index       int      `json:"index"`
	Replacement string   `json:"replacement"`
}
type observed struct {
	data        Vector
	bottom      bool
	comparisons uint64
	swaps       uint64
	trace       uint64
}

func (o *observed) record(kind byte, left, right int) {
	o.trace = (o.trace ^ uint64(kind)) * 1099511628211
	for _, ordinal := range [...]uint64{uint64(left), uint64(right)} {
		for shift := 0; shift < 64; shift += 8 {
			o.trace = (o.trace ^ ((ordinal >> shift) & 255)) * 1099511628211
		}
	}
}
func (o *observed) Len() int { return len(o.data) }
func (o *observed) Less(i, j int) bool {
	o.comparisons++
	o.record(1, o.data[i].Ordinal, o.data[j].Ordinal)
	if o.bottom {
		return vectorByReverseValueHeap(o.data).Less(i, j)
	}
	return vectorByValueHeap(o.data).Less(i, j)
}
func (o *observed) Swap(i, j int) {
	o.swaps++
	o.record(2, o.data[i].Ordinal, o.data[j].Ordinal)
	vectorByValueHeap(o.data).Swap(i, j)
}
func (o *observed) Push(value any) { (*vectorByValueHeap)(&o.data).Push(value) }
func (o *observed) Pop() any       { return (*vectorByValueHeap)(&o.data).Pop() }
func number(text string) float64 {
	bits, err := strconv.ParseUint(text, 16, 64)
	if err != nil {
		panic(err)
	}
	return math.Float64frombits(bits)
}
func boolean(value bool) int {
	if value {
		return 1
	}
	return 0
}
func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	output := json.NewEncoder(os.Stdout)
	for _, request := range requests {
		data := make(Vector, len(request.Values))
		for i, bits := range request.Values {
			data[i] = Sample{number(bits), i}
		}
		o := &observed{data: data, bottom: request.Bottom, trace: 14695981039346656037}
		var target sort.Interface = o
		if request.Reverse {
			target = sort.Reverse(o)
		}
		var result []int
		n := len(data)
		switch request.Action {
		case "sort":
			sort.Sort(target)
		case "insertion":
			insertionSort(target, 0, n)
		case "heap":
			heapSort(target, 0, n)
		case "partial":
			result = []int{boolean(partialInsertionSort(target, 0, n))}
		case "partition":
			mid, partitioned := partition(target, 0, n, request.Pivot)
			result = []int{mid, boolean(partitioned)}
		case "equal":
			result = []int{partitionEqual(target, 0, n, request.Pivot)}
		case "pivot":
			pivot, hint := choosePivot(target, 0, n)
			result = []int{pivot, int(hint)}
		case "reverse":
			reverseRange(target, 0, n)
		case "patterns":
			breakPatterns(target, 0, n)
		case "push", "fix":
			o.data = make(Vector, 0, n)
			for i := range data {
				heap.Push(o, &data[i])
			}
			if request.Action == "fix" {
				o.data[request.Index].F = number(request.Replacement)
				heap.Fix(o, request.Index)
			}
		default:
			panic("unknown ordering action")
		}
		items := make([]map[string]any, len(o.data))
		for i, item := range o.data {
			items[i] = map[string]any{"bits": fmt.Sprintf("%016x", math.Float64bits(item.F)), "ordinal": item.Ordinal}
		}
		if err := output.Encode(map[string]any{"id": request.ID, "actual": map[string]any{
			"items": items, "comparisons": o.comparisons, "swaps": o.swaps,
			"trace": fmt.Sprintf("%016x", o.trace), "result": result,
		}}); err != nil {
			panic(err)
		}
	}
}
