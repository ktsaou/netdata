// SPDX-License-Identifier: GPL-3.0-or-later
// Emit pinned decomposition and stream-safe properties without exposing norm internals.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"unicode"

	"golang.org/x/text/unicode/norm"
)

type entry struct {
	Rune          uint32   `json:"rune"`
	Decomposition []uint32 `json:"decomposition"`
	CCC           uint8    `json:"ccc"`
	Leading       uint8    `json:"leading"`
	Trailing      uint8    `json:"trailing"`
}

func main() {
	if runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" || norm.Version != "17.0.0" {
		fmt.Fprintln(os.Stderr, "normalization tables require go1.27.1 / Unicode 17.0.0")
		os.Exit(1)
	}
	entries := []entry{}
	for r := rune(0); r <= unicode.MaxRune; r++ {
		if r >= 0xd800 && r <= 0xdfff {
			continue
		}
		original := string(r)
		decomposed := []rune(norm.NFKD.String(original))
		leading, trailing := uint8(0), uint8(0)
		for _, d := range decomposed {
			if norm.NFKD.PropertiesString(string(d)).BoundaryBefore() {
				break
			}
			leading++
		}
		for i := len(decomposed) - 1; i >= 0; i-- {
			if norm.NFKD.PropertiesString(string(decomposed[i])).BoundaryBefore() {
				break
			}
			trailing++
		}
		ccc := norm.NFKD.PropertiesString(original).CCC()
		if r >= 0xac00 && r <= 0xd7a3 {
			index := r - 0xac00
			want := []rune{0x1100 + index/588, 0x1161 + (index%588)/28}
			if index%28 != 0 {
				want = append(want, 0x11a7+index%28)
			}
			if string(want) != string(decomposed) || ccc != 0 || leading != 0 || int(trailing) != len(want)-1 {
				panic("Hangul decomposition formula differs from pinned norm")
			}
			continue
		}
		if leading > 0 && leading != trailing || trailing > 3 {
			panic("unexpected pinned stream-safe properties")
		}
		if len(decomposed) == 1 && decomposed[0] == r && ccc == 0 && leading == 0 && trailing == 0 {
			continue
		}
		d := []uint32{}
		if len(decomposed) != 1 || decomposed[0] != r {
			for _, v := range decomposed {
				d = append(d, uint32(v))
			}
		}
		entries = append(entries, entry{uint32(r), d, ccc, leading, trailing})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"go_version": runtime.Version(),
		"unicode_version": unicode.Version, "norm_version": norm.Version, "entries": entries}); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
