// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only hook inside the verified Prometheus labels source facade.
package labels

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"strings"
	"testing"
	"unicode"

	"github.com/grafana/regexp"
)

func TestNetdataCaptureRegexSelectors(t *testing.T) {
	destination := os.Getenv("NETDATA_REGEX_SELECTOR_CAPTURE")
	if destination == "" {
		t.Skip("capture path not supplied")
	}
	if runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		t.Fatal("unexpected reference toolchain")
	}
	t.Run("original-matchers", TestFastRegexMatcher_MatchString)
	if t.Failed() {
		t.Fatal("original matcher assertions failed")
	}
	testValues := append([]string{}, values...)
	testValues = append(testValues, generateRandomValues()...)
	cases := []map[string]any{}
	for patternIndex, pattern := range regexes {
		matcher, err := NewFastRegexMatcher(pattern)
		if err != nil {
			t.Fatal(err)
		}
		compiled := regexp.MustCompile("^(?s:" + pattern + ")$")
		for subjectIndex, subject := range testValues {
			if compiled.MatchString(subject) != matcher.MatchString(subject) {
				t.Fatal("captured wrapper differs from independently compiled expression")
			}
			cases = append(cases, map[string]any{"id": fmt.Sprintf("selector:%03d:%03d", patternIndex, subjectIndex),
				"mode": "selector", "profile": "Prometheus FastRegexMatcher", "pattern": []byte(pattern),
				"subject": []byte(subject), "expected": map[string]any{"error": nil, "matches": matcher.MatchString(subject)}})
		}
	}
	var requests []struct {
		ID       string   `json:"id"`
		Pattern  []byte   `json:"pattern"`
		Subjects [][]byte `json:"subjects"`
	}
	raw, err := os.ReadFile(os.Getenv("NETDATA_REGEX_REQUESTS"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, &requests); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	normalizations := 0
	for _, request := range requests {
		matcher, err := NewFastRegexMatcher(string(request.Pattern))
		for index, subject := range request.Subjects {
			if !seen[string(subject)] {
				seen[string(subject)] = true
				cases = append(cases, map[string]any{"id": fmt.Sprintf("normalization:%04d", normalizations),
					"mode": "normalize", "profile": "Go NFKD and ToLower", "pattern": []byte{}, "subject": subject,
					"expected": map[string]any{"error": nil, "normalized": []byte(toNormalisedLower(string(subject), nil)),
						"lower": []byte(strings.ToLower(string(subject)))}})
				normalizations++
			}
			expected := map[string]any{"error": nil}
			if err != nil {
				expected["error"] = []byte(err.Error())
			} else {
				expected["matches"] = matcher.MatchString(string(subject))
			}
			cases = append(cases, map[string]any{"id": fmt.Sprintf("targeted:selector:%s:%03d", request.ID, index),
				"mode": "selector", "profile": "targeted Prometheus FastRegexMatcher", "pattern": request.Pattern,
				"subject": subject, "expected": expected})
		}
	}
	file, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := json.NewEncoder(file).Encode(map[string]any{"schema_version": 1,
		"commit": "5241a27fe3c6983549fccc32f6e65917408c63cd", "go_version": runtime.Version(),
		"unicode_version": unicode.Version, "original_assertions_passed": true,
		"pattern_count": len(regexes), "subject_count": len(testValues), "cases": cases}); err != nil {
		t.Fatal(err)
	}
}
