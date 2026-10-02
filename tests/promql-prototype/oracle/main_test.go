package main

import (
	"github.com/prometheus/prometheus/tsdb/chunkenc"
	"testing"
)

func TestIteratorForwardSeekAndExhaustion(t *testing.T) {
	it := &floatIterator{ts: []int64{10, 20, 30}, vs: []float64{1, 2, 3}, i: -1}
	if it.Seek(15) != chunkenc.ValFloat || it.AtT() != 20 {
		t.Fatal("seek must find first sample at or after request")
	}
	if it.Seek(5) != chunkenc.ValFloat || it.AtT() != 20 {
		t.Fatal("seek must not rewind")
	}
	if it.Next() != chunkenc.ValFloat || it.AtT() != 30 {
		t.Fatal("next after seek")
	}
	if it.Seek(31) != chunkenc.ValNone || it.Seek(5) != chunkenc.ValNone || it.Next() != chunkenc.ValNone {
		t.Fatal("exhaustion must persist")
	}
}
