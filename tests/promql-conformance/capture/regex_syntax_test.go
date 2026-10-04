// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only hook inside the verified Grafana syntax source facade.
package syntax

import (
	"encoding/json"
	"fmt"
	"os"
	standard "regexp/syntax"
	"runtime"
	"testing"
	"unicode"
)

func TestNetdataCaptureRegexSyntax(t *testing.T) {
	destination := os.Getenv("NETDATA_REGEX_SYNTAX_CAPTURE")
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
		{"simple", TestParseSimple}, {"fold", TestParseFoldCase}, {"literal", TestParseLiteral},
		{"match-newline", TestParseMatchNL}, {"exclude-newline", TestParseNoMatchNL},
		{"invalid", TestParseInvalidRegexps}, {"string", TestString}, {"roundtrip", TestToStringEquivalentParse},
	} {
		t.Run(assertion.name, assertion.run)
	}
	if t.Failed() {
		t.Fatal("original regex syntax assertions failed")
	}
	cases := []map[string]any{}
	record := func(id, profile, pattern string, flags Flags) {
		for _, standardProfile := range []bool{false, true} {
			var re interface {
				String() string
				CapNames() []string
			}
			var err error
			mode, caseID, origin := "syntax", id, "github.com/grafana/regexp/syntax"
			if standardProfile {
				re, err = standard.Parse(pattern, standard.Flags(flags))
				mode, caseID, origin = "standard-syntax", "std:"+id, "Go 1.27.1 regexp/syntax"
			} else {
				re, err = Parse(pattern, flags)
			}
			expected := map[string]any{"error": nil}
			if err != nil {
				var code, expression string
				switch syntaxError := err.(type) {
				case *Error:
					code, expression = string(syntaxError.Code), syntaxError.Expr
				case *standard.Error:
					code, expression = string(syntaxError.Code), syntaxError.Expr
				default:
					t.Fatalf("unexpected syntax error type: %T", err)
				}
				expected["error"] = map[string]any{"code": []byte(code),
					"expression": []byte(expression), "message": []byte(err.Error())}
			} else {
				expected["canonical"] = []byte(re.String())
				names := make([][]byte, len(re.CapNames()))
				for index, name := range re.CapNames() {
					names[index] = []byte(name)
				}
				expected["names"] = names
			}
			cases = append(cases, map[string]any{"id": caseID, "mode": mode, "profile": profile, "origin": origin,
				"pattern": []byte(pattern), "flags": flags, "expected": expected})
		}
	}
	for _, group := range []struct {
		name  string
		rows  []parseTest
		flags Flags
	}{
		{"simple", parseTests, testFlags}, {"fold", foldcaseTests, FoldCase},
		{"literal", literalTests, Literal}, {"match-newline", matchnlTests, MatchNL},
		{"exclude-newline", nomatchnlTests, 0},
	} {
		for index, row := range group.rows {
			record(fmt.Sprintf("syntax:%s:%03d", group.name, index), group.name, row.Regexp, group.flags)
		}
	}
	for _, group := range []struct {
		name string
		rows []string
	}{
		{"invalid", invalidRegexps}, {"perl-only", onlyPerl}, {"posix-only", onlyPOSIX},
	} {
		for index, pattern := range group.rows {
			for _, flags := range []Flags{Perl, POSIX} {
				record(fmt.Sprintf("syntax:%s:%03d:%03d", group.name, index, flags),
					fmt.Sprintf("flags-%d", flags), pattern, flags)
			}
		}
	}
	for index, row := range stringTests {
		record(fmt.Sprintf("syntax:string:%03d", index), "string", row.re, Perl)
	}
	var requests []struct {
		ID      string   `json:"id"`
		Pattern []byte   `json:"pattern"`
		Flags   []uint16 `json:"flags"`
	}
	raw, err := os.ReadFile(os.Getenv("NETDATA_REGEX_REQUESTS"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, &requests); err != nil {
		t.Fatal(err)
	}
	for _, request := range requests {
		for _, flags := range request.Flags {
			record(fmt.Sprintf("syntax:targeted:%s:%03d", request.ID, flags), "targeted", string(request.Pattern), Flags(flags))
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
		"syntax_origin": "github.com/grafana/regexp/syntax", "cases": cases}); err != nil {
		t.Fatal(err)
	}
}
