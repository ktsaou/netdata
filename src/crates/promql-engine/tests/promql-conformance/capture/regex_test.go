// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only hook inside the verified Grafana regexp source facade.
package regexp

import (
	"encoding/json"
	"fmt"
	"os"
	"runtime"
	"testing"
	"unicode"
)

func TestNetdataCaptureRegex(t *testing.T) {
	destination := os.Getenv("NETDATA_REGEX_CAPTURE")
	if destination == "" {
		t.Skip("capture path not supplied")
	}
	if runtime.Version() != "go1.27.1" || unicode.Version != "17.0.0" {
		t.Fatal("unexpected reference toolchain")
	}
	for _, assertion := range []struct {
		name string
		run  func(*testing.T)
	}{
		{"good-compile", TestGoodCompile}, {"bad-compile", TestBadCompile},
		{"first-captures", TestFindStringSubmatchIndex}, {"all-captures", TestFindAllStringSubmatchIndex},
		{"replacement", TestReplaceAll}, {"metadata", TestSubexp},
		{"onepass-compile", TestCompileOnePass}, {"onepass-execution", TestRunOnePass},
	} {
		t.Run(assertion.name, assertion.run)
	}
	if t.Failed() {
		t.Fatal("original regex assertions failed")
	}
	cases := []map[string]any{}
	recordProgram := func(id, pattern string) {
		re, err := Compile(pattern)
		expected := map[string]any{"error": nil}
		if err != nil {
			expected["error"] = []byte(err.Error())
		} else {
			instructions := []map[string]any{}
			for _, instruction := range re.prog.Inst {
				instructions = append(instructions, map[string]any{"op": instruction.Op.String(),
					"out": instruction.Out, "arg": instruction.Arg, "runes": instruction.Rune})
			}
			expected["start"], expected["num_cap"] = re.prog.Start, re.prog.NumCap
			expected["one_pass"], expected["instructions"] = re.onepass != nil, instructions
			expected["prefix"], expected["prefix_end"], expected["prefix_complete"] = []byte(re.prefix), re.prefixEnd, re.prefixComplete
		}
		cases = append(cases, map[string]any{"id": id, "mode": "program", "profile": "Go compileOnePass",
			"pattern": []byte(pattern), "expected": expected})
	}
	record := func(id, mode, pattern, subject string, templates, lookups []string) {
		compiledPattern := pattern
		if mode == "label_replace" {
			compiledPattern = "^(?s:" + pattern + ")$"
		}
		re, err := Compile(compiledPattern)
		expected := map[string]any{"error": nil}
		if err != nil {
			expected["error"] = []byte(err.Error())
		} else {
			expected["num_subexp"] = re.NumSubexp()
			names := make([][]byte, len(re.SubexpNames()))
			for index, name := range re.SubexpNames() {
				names[index] = []byte(name)
			}
			expected["names"] = names
			indices := make([]int, len(lookups))
			for index, name := range lookups {
				indices[index] = re.SubexpIndex(name)
			}
			expected["indices"] = indices
			if mode == "find" || mode == "label_replace" {
				expected["match_string"] = re.MatchString(subject)
				expected["first"] = re.FindStringSubmatchIndex(subject)
				matches := re.FindAllStringSubmatchIndex(subject, -1)
				expected["matches"] = matches
				expansions := make([][][]byte, len(templates))
				replacements := make([][]byte, len(templates))
				for index, template := range templates {
					replacements[index] = []byte(re.ReplaceAllString(subject, template))
					expansions[index] = make([][]byte, len(matches))
					for matchIndex, match := range matches {
						expansions[index][matchIndex] = re.ExpandString([]byte{}, template, subject, match)
					}
				}
				expected["expansions"] = expansions
				expected["replacements"] = replacements
			}
		}
		byteTemplates := make([][]byte, len(templates))
		for index, template := range templates {
			byteTemplates[index] = []byte(template)
		}
		byteLookups := make([][]byte, len(lookups))
		for index, name := range lookups {
			byteLookups[index] = []byte(name)
		}
		cases = append(cases, map[string]any{"id": id, "mode": mode, "profile": "leftmost-first",
			"pattern": []byte(pattern), "subject": []byte(subject), "templates": byteTemplates,
			"lookups": byteLookups, "expected": expected})
	}
	for index, pattern := range goodRe {
		record(fmt.Sprintf("compile:good:%03d", index), "compile", pattern, "", nil, nil)
		recordProgram(fmt.Sprintf("program:good:%03d", index), pattern)
	}
	for index, row := range badRe {
		record(fmt.Sprintf("compile:bad:%03d", index), "compile", row.re, "", nil, nil)
		recordProgram(fmt.Sprintf("program:bad:%03d", index), row.re)
	}
	for index, row := range findTests {
		record(fmt.Sprintf("find:%03d", index), "find", row.pat, row.text, nil, nil)
	}
	for index, row := range replaceTests {
		record(fmt.Sprintf("replace:%03d", index), "find", row.pattern, row.input, []string{row.replacement}, nil)
	}
	for index, row := range subexpCases {
		lookups := make([]string, len(row.indices))
		for index, lookup := range row.indices {
			lookups[index] = lookup.name
		}
		record(fmt.Sprintf("metadata:%03d", index), "compile", row.input, "", nil, lookups)
	}
	var requests []struct {
		ID        string   `json:"id"`
		Pattern   []byte   `json:"pattern"`
		Subjects  [][]byte `json:"subjects"`
		Templates [][]byte `json:"templates"`
		Lookups   [][]byte `json:"lookups"`
	}
	raw, err := os.ReadFile(os.Getenv("NETDATA_REGEX_REQUESTS"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, &requests); err != nil {
		t.Fatal(err)
	}
	for _, request := range requests {
		recordProgram("program:targeted:find:"+request.ID, string(request.Pattern))
		recordProgram("program:targeted:label_replace:"+request.ID, "^(?s:"+string(request.Pattern)+")$")
		templates := make([]string, len(request.Templates))
		for index, template := range request.Templates {
			templates[index] = string(template)
		}
		lookups := make([]string, len(request.Lookups))
		for index, lookup := range request.Lookups {
			lookups[index] = string(lookup)
		}
		for index, subject := range request.Subjects {
			for _, mode := range []string{"find", "label_replace"} {
				record(fmt.Sprintf("targeted:%s:%s:%03d", mode, request.ID, index), mode, string(request.Pattern), string(subject), templates, lookups)
			}
		}
	}
	file, err := os.OpenFile(destination, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := json.NewEncoder(file).Encode(map[string]any{"schema_version": 1,
		"commit": "f7b3be9d18538c56fb03caa6088db55404e784e8", "go_version": runtime.Version(),
		"unicode_version": unicode.Version, "original_assertions_passed": true,
		"syntax_origin": "Go 1.27.1 regexp/syntax", "cases": cases}); err != nil {
		t.Fatal(err)
	}
}
