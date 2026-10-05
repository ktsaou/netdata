// SPDX-License-Identifier: GPL-3.0-or-later
package main

import (
	"math"

	"github.com/prometheus/prometheus/promql/parser"
)

func integerValue(value float64) bool {
	return math.IsNaN(value) || math.Trunc(value) == value
}

func unitLiteral(expr parser.Expr) bool {
	switch n := expr.(type) {
	case *parser.NumberLiteral:
		return n.Val == 1 || n.Val == -1
	case *parser.ParenExpr:
		return unitLiteral(n.Expr)
	case *parser.UnaryExpr:
		return unitLiteral(n.Expr)
	}
	return false
}

// This independent fixture contract follows value semantics, never observed candidate values.
func exactNumeric(expr parser.Expr) bool {
	switch n := expr.(type) {
	case *parser.NumberLiteral:
		return integerValue(n.Val)
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
		left, right := exactNumeric(n.LHS), exactNumeric(n.RHS)
		if n.VectorMatching != nil {
			if fill := n.VectorMatching.FillValues.LHS; fill != nil {
				left = left && integerValue(*fill)
			}
			if fill := n.VectorMatching.FillValues.RHS; fill != nil {
				right = right && integerValue(*fill)
			}
		}
		if n.Op.IsComparisonOperator() {
			if n.LHS.Type() == parser.ValueTypeScalar && n.RHS.Type() == parser.ValueTypeVector {
				return right
			}
			return left
		}
		switch n.Op {
		case parser.ADD, parser.SUB, parser.MUL, parser.MOD:
			return left && right
		case parser.LAND, parser.LUNLESS:
			return left
		case parser.LOR:
			return left && right
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
		case "absent", "absent_over_time", "count_over_time", "present_over_time", "changes", "resets",
			"sgn", "ceil", "floor", "days_in_month", "day_of_month", "day_of_week", "day_of_year",
			"hour", "minute", "month", "year":
			return true
		case "round":
			// Dividing by an inexact reciprocal can produce fractions even for integer steps.
			return len(n.Args) == 1 || len(n.Args) == 2 && unitLiteral(n.Args[1])
		case "abs", "scalar", "vector", "sort", "sort_desc", "sort_by_label", "sort_by_label_desc",
			"label_join", "label_replace", "info", "first_over_time", "last_over_time", "min_over_time", "max_over_time", "sum_over_time":
			return len(n.Args) > 0 && exactNumeric(n.Args[0])
		}
	}
	return false
}
