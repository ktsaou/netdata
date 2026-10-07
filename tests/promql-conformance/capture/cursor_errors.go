// SPDX-License-Identifier: GPL-3.0-or-later
// Construction failures have no prepared or evaluated query scope.
package main

func constructionRecord(r request, events []phaseEvent, raw []map[string]any, err error) map[string]any {
	value := result{ID: r.ID, Kind: "error", Rows: []series{}, Warnings: [][]byte{}, Infos: [][]byte{},
		Error: encodeError("construction", err)}
	return map[string]any{
		"request": r, "result": value, "events": events,
		"raw_ast": raw, "prepared_ast": []map[string]any{}, "executed_ast": []map[string]any{},
		"header": map[string]any{},
	}
}
