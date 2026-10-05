// SPDX-License-Identifier: GPL-3.0-or-later
// Request preparation derives from pinned PromQL durationVisitor/context folding; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    evaluator::EvaluationError,
    frontend::{
        ast::{Expr, NodeId, ParsedExpression, SelectorModifiers, ValueType},
        number, token as tokens,
    },
    numerical,
    result::OutputHeader,
    storage::{Control, Interrupted},
};
use std::fmt::Write;
use tokens::TokenKind;

#[derive(Clone, Copy, Debug)]
pub(crate) struct QueryContext {
    pub(crate) start_ms: i64,
    pub(crate) end_ms: i64,
    step_ns: i64,
    range_ns: i64,
}
impl QueryContext {
    fn new(header: OutputHeader) -> Self {
        let range = (i128::from(header.end_ms) - i128::from(header.start_ms)) * 1_000_000;
        Self {
            start_ms: header.start_ms,
            end_ms: header.end_ms,
            step_ns: header.step_ms.wrapping_mul(1_000_000),
            range_ns: range.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
        }
    }
    pub(crate) fn scalar(self, name: &str) -> Option<f64> {
        Some(match name {
            "start" => self.start_ms as f64 / 1000.0,
            "end" => self.end_ms as f64 / 1000.0,
            "range" => number::seconds(self.range_ns),
            "step" if self.start_ms != self.end_ms => number::seconds(self.step_ns),
            "step" => 0.0,
            _ => return None,
        })
    }
}

const RANGE_SLOT: usize = 0;
const STEP_SLOT: usize = 1;
const OFFSET_SLOT: usize = 2;

#[derive(Clone, Copy, Default)]
pub(crate) struct SelectorReference {
    pub(crate) timestamp_ms: Option<i64>,
    pub(crate) offset_ns: i64,
}
impl SelectorReference {
    fn equal(self, other: Self) -> bool {
        match (self.timestamp_ms, other.timestamp_ms) {
            (None, None) => self.offset_ns / 1_000_000 == other.offset_ns / 1_000_000,
            (Some(left), Some(right)) => {
                left.wrapping_sub(self.offset_ns / 1_000_000)
                    == right.wrapping_sub(other.offset_ns / 1_000_000)
            }
            _ => false,
        }
    }
}

pub(crate) struct Prepared {
    pub(crate) context: QueryContext,
    pub(crate) header: OutputHeader,
    durations: Option<AccountedVec<[i64; 3]>>,
}
impl Prepared {
    pub(crate) fn new(
        expression: &ParsedExpression,
        header: OutputHeader,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let context = QueryContext::new(header);
        // Evaluation uses the nanosecond clock; preprocessing retains Unix milliseconds.
        let header = OutputHeader {
            start_ms: header.start_ms.wrapping_mul(1_000_000) / 1_000_000,
            end_ms: header.end_ms.wrapping_mul(1_000_000) / 1_000_000,
            ..header
        };
        let mut needed = false;
        for node in &expression.arena.nodes {
            control.check()?;
            account.work(1);
            needed |= match &node.expression {
                Expr::Selector(selector) => selector.modifiers.original_offset_expression.is_some(),
                Expr::Matrix {
                    range_expression, ..
                } => range_expression.is_some(),
                Expr::Subquery {
                    range_expression,
                    step_expression,
                    modifiers,
                    ..
                } => {
                    range_expression.is_some()
                        || step_expression.is_some()
                        || modifiers.original_offset_expression.is_some()
                }
                _ => false,
            };
        }
        let Some(mut durations) = needed.then(|| AccountedVec::new(account, MemoryKind::Workspace))
        else {
            return Ok(Self {
                context,
                header,
                durations: None,
            });
        };
        for node in &expression.arena.nodes {
            control.check()?;
            account.work(1);
            durations.push(match &node.expression {
                Expr::Selector(selector) => [0, 0, selector.modifiers.original_offset],
                Expr::Matrix { range, .. } => [*range, 0, 0],
                Expr::Subquery {
                    range,
                    step,
                    modifiers,
                    ..
                } => [*range, *step, modifiers.original_offset],
                _ => [0; 3],
            });
        }
        let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
        tasks.push(expression.root);
        while let Some(node) = tasks.pop() {
            control.check()?;
            account.work(1);
            let value = &expression.arena[node].expression;
            match value {
                Expr::Selector(selector) => {
                    if let Some(offset) = selector.modifiers.original_offset_expression {
                        durations[node.0][OFFSET_SLOT] =
                            duration(expression, offset, true, context, control, account)?;
                    }
                }
                Expr::Matrix {
                    range_expression: Some(range),
                    ..
                } => {
                    durations[node.0][RANGE_SLOT] =
                        duration(expression, *range, false, context, control, account)?;
                }
                Expr::Subquery {
                    range_expression,
                    step_expression,
                    modifiers,
                    ..
                } => {
                    if let Some(offset) = modifiers.original_offset_expression {
                        durations[node.0][OFFSET_SLOT] =
                            duration(expression, offset, true, context, control, account)?;
                    }
                    if let Some(step) = step_expression {
                        durations[node.0][STEP_SLOT] =
                            duration(expression, *step, false, context, control, account)?;
                    }
                    if let Some(range) = range_expression {
                        durations[node.0][RANGE_SLOT] =
                            duration(expression, *range, false, context, control, account)?;
                    }
                }
                _ => (),
            }
            tasks.extend(value.children().rev());
        }
        Ok(Self {
            context,
            header,
            durations: Some(durations),
        })
    }
    pub(crate) fn range(&self, node: NodeId, value: i64) -> i64 {
        self.durations
            .as_ref()
            .map_or(value, |values| values[node.0][RANGE_SLOT])
    }
    pub(crate) fn offset(&self, node: NodeId, value: i64) -> i64 {
        self.durations
            .as_ref()
            .map_or(value, |values| values[node.0][OFFSET_SLOT])
    }
    pub(crate) fn timestamp(&self, modifiers: &SelectorModifiers) -> Option<i64> {
        modifiers.timestamp.or(match modifiers.start_or_end {
            Some(TokenKind::START) => Some(self.context.start_ms),
            Some(TokenKind::END) => Some(self.context.end_ms),
            _ => None,
        })
    }

    pub(crate) fn adjusted_offset(
        &self,
        node: NodeId,
        modifiers: &SelectorModifiers,
        scope_start_ms: i64,
    ) -> i64 {
        let offset = self.offset(node, modifiers.original_offset);
        if let Some(timestamp) = self.timestamp(modifiers) {
            offset.wrapping_add(
                scope_start_ms
                    .wrapping_sub(timestamp)
                    .wrapping_mul(1_000_000),
            )
        } else {
            offset
        }
    }

    // Scalar paths and selector-free vector branches must not supply an info-series reference.
    pub(crate) fn info_reference(
        &self,
        expression: &ParsedExpression,
        root: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<SelectorReference, Interrupted> {
        enum Task {
            Visit(NodeId, SelectorReference),
            Finish(usize, bool),
        }
        let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
        let mut values = AccountedVec::new(account, MemoryKind::Workspace);
        tasks.push(Task::Visit(root, SelectorReference::default()));
        let mut first = None::<SelectorReference>;
        let mut uniform = true;
        let mut reference_free = false;
        while let Some(task) = tasks.pop() {
            control.check()?;
            account.work(1);
            match task {
                Task::Finish(base, check_reference_free) => {
                    let mut has_selector = false;
                    while values.len() > base {
                        control.check()?;
                        account.work(1);
                        has_selector |= values.pop().unwrap();
                    }
                    reference_free |= check_reference_free && !has_selector;
                    values.push(has_selector);
                }
                Task::Visit(node, mut scope) => {
                    if !matches!(
                        expression.arena[node].value_type,
                        ValueType::Vector | ValueType::Matrix
                    ) {
                        values.push(false);
                        continue;
                    }
                    let value = &expression.arena[node].expression;
                    if let Expr::Selector(selector) = value {
                        let timestamp_ms = self.timestamp(&selector.modifiers);
                        let offset_ns = self.offset(node, selector.modifiers.original_offset);
                        let reference = if timestamp_ms.is_some() {
                            SelectorReference {
                                timestamp_ms,
                                offset_ns,
                            }
                        } else {
                            SelectorReference {
                                timestamp_ms: scope.timestamp_ms,
                                offset_ns: offset_ns.wrapping_add(scope.offset_ns),
                            }
                        };
                        if let Some(first) = first {
                            uniform &= first.equal(reference);
                        } else {
                            first = Some(reference);
                        }
                        values.push(true);
                        continue;
                    }
                    if let Expr::Subquery { modifiers, .. } = value {
                        let offset = self.offset(node, modifiers.original_offset);
                        scope.offset_ns = scope.offset_ns.wrapping_add(offset);
                        if let Some(timestamp) = self.timestamp(modifiers) {
                            scope.timestamp_ms = Some(timestamp);
                            scope.offset_ns = offset;
                        }
                    }
                    if let Expr::Call {
                        function: Some(function),
                        arguments,
                    } = value
                        && function.name == "info"
                    {
                        tasks.push(Task::Finish(values.len(), false));
                        tasks.push(Task::Visit(arguments[0], scope));
                    } else {
                        tasks.push(Task::Finish(values.len(), true));
                        tasks.extend(
                            value
                                .children()
                                .rev()
                                .map(|child| Task::Visit(child, scope)),
                        );
                    }
                }
            }
        }
        Ok(if uniform && !reference_free {
            first.unwrap_or_default()
        } else {
            SelectorReference::default()
        })
    }

    #[cfg(feature = "fixtures")]
    pub(crate) fn snapshot(&self, expression: &ParsedExpression) -> serde_json::Value {
        use serde_json::{Value, json};
        let span = |node: NodeId| json!({"Start": expression.arena[node].span.start, "End": expression.arena[node].span.end});
        let mut nodes = Vec::new();
        let mut tasks = vec![expression.root];
        while let Some(node) = tasks.pop() {
            let value = &expression.arena[node].expression;
            let mut row = json!({"span": span(node)});
            match value {
                Expr::Selector(selector) => {
                    row["type"] = json!("*parser.VectorSelector");
                    row["offset_ns"] = json!(self.offset(node, selector.modifiers.original_offset));
                    row["timestamp_ms"] = json!(self.timestamp(&selector.modifiers));
                    if let Some(duration) = selector.modifiers.original_offset_expression {
                        row["duration_span"] = span(duration);
                    }
                }
                Expr::Matrix {
                    range,
                    range_expression,
                    ..
                } => {
                    row["type"] = json!("*parser.MatrixSelector");
                    row["range_ns"] = json!(self.range(node, *range));
                    if let Some(duration) = range_expression {
                        row["duration_span"] = span(*duration);
                    }
                }
                Expr::Subquery {
                    range,
                    step,
                    range_expression,
                    step_expression,
                    modifiers,
                    ..
                } => {
                    row["type"] = json!("*parser.SubqueryExpr");
                    row["range_ns"] = json!(self.range(node, *range));
                    row["step_ns"] = json!(
                        self.durations
                            .as_ref()
                            .map_or(*step, |values| values[node.0][STEP_SLOT])
                    );
                    row["offset_ns"] = json!(self.offset(node, modifiers.original_offset));
                    row["timestamp_ms"] = json!(self.timestamp(modifiers));
                    for (field, child) in [
                        ("range_span", *range_expression),
                        ("step_span", *step_expression),
                        ("offset_span", modifiers.original_offset_expression),
                    ] {
                        if let Some(child) = child {
                            row[field] = span(child);
                        }
                    }
                }
                Expr::Number { value, .. } => {
                    row["type"] = json!("*parser.NumberLiteral");
                    row["number"] = json!(format!("{:016x}", value.to_bits()));
                }
                Expr::Call {
                    function: Some(function),
                    ..
                } => {
                    if let Some(value) = self.context.scalar(function.name) {
                        row["type"] = json!("*parser.NumberLiteral");
                        row["number"] = json!(format!("{:016x}", value.to_bits()));
                    }
                }
                _ => (),
            }
            if row["type"] != Value::Null {
                nodes.push(row);
            }
            tasks.extend(value.children().rev());
        }
        json!({"nodes": nodes})
    }
}

fn failure(
    expression: &ParsedExpression,
    node: NodeId,
    message: &str,
    account: &QueryAccounting,
) -> EvaluationError {
    let span = expression.arena[node].span;
    let mut output = AccountedVec::new(account, MemoryKind::Result);
    write!(output, "{}:{}: {message}", span.start, span.end).expect("byte writer");
    EvaluationError::Preparation(output)
}

fn duration(
    expression: &ParsedExpression,
    root: NodeId,
    allowed_negative: bool,
    context: QueryContext,
    control: &Control,
    account: &QueryAccounting,
) -> Result<i64, EvaluationError> {
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    let mut values = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((root, false));
    while let Some((node, finish)) = tasks.pop() {
        control.check()?;
        account.work(1);
        match &expression.arena[node].expression {
            Expr::Number { value, .. } => values.push(*value),
            Expr::Duration {
                operator,
                left,
                right,
                ..
            } => {
                if !finish {
                    tasks.push((node, true));
                    if let Some(right) = right {
                        tasks.push((*right, false));
                    }
                    if let Some(left) = left {
                        tasks.push((*left, false));
                    }
                    continue;
                }
                let rhs = right.map_or(0.0, |_| values.pop().expect("right duration"));
                let lhs = left.map_or(0.0, |_| values.pop().expect("left duration"));
                use TokenKind::*;
                let value = match operator {
                    STEP => number::seconds(context.step_ns),
                    RANGE => number::seconds(context.range_ns),
                    MIN_OF => numerical::min(lhs, rhs),
                    MAX_OF => numerical::max(lhs, rhs),
                    ADD if left.is_none() => rhs,
                    ADD => lhs + rhs,
                    SUB if left.is_none() => -rhs,
                    SUB => lhs - rhs,
                    MUL => lhs * rhs,
                    DIV | MOD if rhs == 0.0 => {
                        return Err(failure(
                            expression,
                            node,
                            if *operator == DIV {
                                "division by zero"
                            } else {
                                "modulo by zero"
                            },
                            account,
                        ));
                    }
                    DIV => lhs / rhs,
                    MOD | POW => {
                        let check = || {
                            account.work(1);
                            control.check()
                        };
                        if *operator == MOD {
                            numerical::modulo_with(lhs, rhs, check)?
                        } else {
                            numerical::pow_with(lhs, rhs, check)?
                        }
                    }
                    _ => unreachable!("validated duration operator"),
                };
                values.push(value);
            }
            _ => unreachable!("validated duration expression"),
        }
    }
    let value = values.pop().expect("duration output");
    if !value.is_finite() {
        return Err(failure(
            expression,
            root,
            "duration is NaN or infinite",
            account,
        ));
    }
    if value <= 0.0 && !allowed_negative {
        return Err(failure(
            expression,
            root,
            "duration must be greater than 0",
            account,
        ));
    }
    if number::duration_out_of_range(value) {
        return Err(failure(
            expression,
            root,
            "duration is out of range",
            account,
        ));
    }
    Ok(number::go_i64(value * 1000.0).wrapping_mul(1_000_000))
}
