// SPDX-License-Identifier: GPL-3.0-or-later
package main

import "github.com/prometheus/prometheus/promql/parser"

// This independent fixture contract follows value semantics, never observed candidate values.
func exactNumeric(expr parser.Expr) bool {
	switch n := expr.(type) {
	case *parser.ParenExpr:
		return exactNumeric(n.Expr)
	case *parser.UnaryExpr:
		return exactNumeric(n.Expr)
	case *parser.SubqueryExpr:
		return exactNumeric(n.Expr)
	case *parser.BinaryExpr:
		if n.ReturnBool {
			return true
		}
		if n.Op.IsComparisonOperator() {
			if n.LHS.Type() == parser.ValueTypeScalar && n.RHS.Type() == parser.ValueTypeVector {
				return exactNumeric(n.RHS)
			}
			return exactNumeric(n.LHS)
		}
		switch n.Op {
		case parser.ADD, parser.SUB, parser.MUL, parser.MOD:
			return exactNumeric(n.LHS) && exactNumeric(n.RHS)
		case parser.LAND, parser.LUNLESS:
			return exactNumeric(n.LHS)
		case parser.LOR:
			return exactNumeric(n.LHS) && exactNumeric(n.RHS)
		}
	case *parser.AggregateExpr:
		switch n.Op {
		case parser.COUNT, parser.COUNT_VALUES, parser.GROUP:
			return true
		case parser.SUM, parser.MIN, parser.MAX, parser.TOPK, parser.BOTTOMK, parser.LIMITK, parser.LIMIT_RATIO:
			return exactNumeric(n.Expr)
		}
	case *parser.Call:
		switch n.Func.Name {
		case "absent", "absent_over_time", "count_over_time", "present_over_time", "changes", "resets":
			return true
		case "abs", "scalar", "vector", "sort", "sort_desc", "sort_by_label", "sort_by_label_desc",
			"label_join", "label_replace", "info", "first_over_time", "last_over_time", "min_over_time", "max_over_time", "sum_over_time":
			return len(n.Args) > 0 && exactNumeric(n.Args[0])
		}
	}
	return false
}
