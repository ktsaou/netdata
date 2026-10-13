// SPDX-License-Identifier: GPL-3.0-or-later
// Emit the pinned Go diagnostic-printability table without downloading dependencies.
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strconv"
	"unicode"
)

func main() {
	if runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		fmt.Fprintln(os.Stderr, "Go diagnostic tables require go1.27.1 / Unicode 17.0.0")
		os.Exit(1)
	}
	ranges := [][2]int32{}
	var start int32 = -1
	for r := int32(0); r <= unicode.MaxRune+1; r++ {
		printable := r <= unicode.MaxRune && strconv.IsPrint(r)
		if printable && start == -1 {
			start = r
		}
		if !printable && start != -1 {
			ranges = append(ranges, [2]int32{start, r - 1})
			start = -1
		}
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"go_version": runtime.Version(), "unicode_version": unicode.Version, "print_ranges": ranges}); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
