// SPDX-License-Identifier: GPL-3.0-or-later
// Emit the pinned Go Unicode data used by the owned regex frontend.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"slices"
	"unicode"
)

type table struct {
	Name   string      `json:"name"`
	Ranges [][3]uint32 `json:"ranges"`
}

func tables(source map[string]*unicode.RangeTable) []table {
	names := make([]string, 0, len(source))
	for name := range source {
		names = append(names, name)
	}
	slices.Sort(names)
	result := make([]table, 0, len(names))
	for _, name := range names {
		ranges := make([][3]uint32, 0)
		for _, r := range source[name].R16 {
			ranges = append(ranges, [3]uint32{uint32(r.Lo), uint32(r.Hi), uint32(r.Stride)})
		}
		for _, r := range source[name].R32 {
			ranges = append(ranges, [3]uint32{r.Lo, r.Hi, r.Stride})
		}
		result = append(result, table{name, ranges})
	}
	return result
}

func main() {
	if runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		fmt.Fprintln(os.Stderr, "regex tables require go1.27.1 / Unicode 17.0.0")
		os.Exit(1)
	}
	folds := make([][2]uint32, 0)
	lower := make([][2]uint32, 0)
	for r := rune(0); r <= unicode.MaxRune; r++ {
		if next := unicode.SimpleFold(r); next != r {
			folds = append(folds, [2]uint32{uint32(r), uint32(next)})
		}
		if next := unicode.ToLower(r); next != r {
			lower = append(lower, [2]uint32{uint32(r), uint32(next)})
		}
	}
	result := map[string]any{
		"go_version": runtime.Version(), "unicode_version": unicode.Version,
		"categories": tables(unicode.Categories), "scripts": tables(unicode.Scripts),
		"fold_categories": tables(unicode.FoldCategory), "fold_scripts": tables(unicode.FoldScript),
		"simple_fold":      folds,
		"lower":            lower,
		"category_aliases": unicode.CategoryAliases,
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
