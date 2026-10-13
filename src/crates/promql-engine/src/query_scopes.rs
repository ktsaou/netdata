// SPDX-License-Identifier: GPL-3.0-or-later
// Evaluator grids and offset binding derive from pinned promql/engine.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::ast::{Expr, NodeId, ParsedExpression, SelectorModifiers},
    query_preparation::Prepared,
    result::OutputHeader,
    storage::{Control, Interrupted},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Grid {
    pub(crate) start_ms: i64,
    pub(crate) end_ms: i64,
    pub(crate) interval_ms: i64,
}
impl Grid {
    pub(crate) fn forward_without_wrap(self) -> bool {
        self.interval_ms > 0 && self.end_ms.checked_add(self.interval_ms).is_some()
    }

    pub(crate) fn steps(self) -> Result<i64, GridFault> {
        Ok(quotient(self.end_ms.wrapping_sub(self.start_ms), self.interval_ms)?.wrapping_add(1))
    }

    pub(crate) fn header(self, parent: OutputHeader) -> OutputHeader {
        OutputHeader {
            start_ms: self.start_ms,
            end_ms: self.end_ms,
            step_ms: self.interval_ms,
            ..parent
        }
    }

    fn subquery(
        self,
        range_ns: i64,
        step_ns: i64,
        offset_ns: i64,
        no_step_interval_ms: i64,
    ) -> Result<Self, GridFault> {
        let offset_ms = offset_ns / 1_000_000;
        let range_ms = range_ns / 1_000_000;
        let interval_ms = if step_ns != 0 {
            step_ns / 1_000_000
        } else {
            no_step_interval_ms
        };
        // Geometry is recorded here; its source execution phase reports any fault.
        if interval_ms == 0 {
            return Err(GridFault::ZeroInterval);
        }
        let parent_end = if self.interval_ms > 0 {
            self.start_ms.wrapping_add(
                (self.end_ms.wrapping_sub(self.start_ms) / self.interval_ms)
                    .wrapping_mul(self.interval_ms),
            )
        } else {
            self.end_ms
        };
        let lower = self.start_ms.wrapping_sub(offset_ms).wrapping_sub(range_ms);
        let mut start_ms = interval_ms.wrapping_mul(quotient(lower, interval_ms)?);
        if start_ms <= lower {
            start_ms = start_ms.wrapping_add(interval_ms);
        }
        let grid = Self {
            start_ms,
            end_ms: parent_end.wrapping_sub(offset_ms),
            interval_ms,
        };
        if grid.steps()? < 0 {
            return Err(GridFault::NegativeStepCount);
        }
        Ok(grid)
    }
}

pub(crate) struct Traversal {
    grid: Grid,
    current: Option<(i64, i64)>,
    shortcut: bool,
}
impl Traversal {
    pub(crate) fn new(grid: Grid, shortcut: bool) -> Self {
        Self {
            current: (grid.start_ms <= grid.end_ms).then_some((grid.start_ms, 0)),
            grid,
            shortcut,
        }
    }
    pub(crate) fn current(&self) -> Option<(i64, i64)> {
        self.current
    }
    pub(crate) fn advance(&mut self) {
        let Some((time_ms, index)) = self.current else {
            return;
        };
        if self.shortcut && self.grid.start_ms == self.grid.end_ms {
            self.current = None;
            return;
        }
        let next = time_ms.wrapping_add(self.grid.interval_ms);
        self.current = (next <= self.grid.end_ms).then_some((next, index.wrapping_add(1)));
    }
}

pub(crate) fn quotient(numerator: i64, denominator: i64) -> Result<i64, GridFault> {
    if denominator == 0 {
        return Err(GridFault::ZeroInterval);
    }
    // Go's signed MIN/-1 division retains MIN without a runtime panic.
    Ok(if numerator == i64::MIN && denominator == -1 {
        i64::MIN
    } else {
        numerator / denominator
    })
}

impl From<OutputHeader> for Grid {
    fn from(header: OutputHeader) -> Self {
        Self {
            start_ms: header.start_ms,
            end_ms: header.end_ms,
            interval_ms: header.step_ms,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GridFault {
    ZeroInterval,
    NegativeStepCount,
    NegativeCapacity,
}
impl GridFault {
    pub(crate) fn message(self) -> &'static [u8] {
        match self {
            Self::ZeroInterval => b"unexpected error: runtime error: integer divide by zero",
            Self::NegativeStepCount => {
                b"unexpected error: runtime error: makeslice: len out of range"
            }
            Self::NegativeCapacity => {
                b"unexpected error: runtime error: makeslice: cap out of range"
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Binding {
    epoch_ms: i64,
    ancestor_offset_ns: i64,
    ancestor_timestamp_ms: Option<i64>,
}
impl Binding {
    fn new(epoch_ms: i64) -> Self {
        Self {
            epoch_ms,
            ancestor_offset_ns: 0,
            ancestor_timestamp_ms: None,
        }
    }
    fn offset(self, original_ns: i64, timestamp_ms: Option<i64>) -> i64 {
        let Some(timestamp_ms) = timestamp_ms else {
            return original_ns;
        };
        let mut ancestor = self.ancestor_offset_ns;
        if let Some(timestamp) = self.ancestor_timestamp_ms {
            ancestor = ancestor.wrapping_add(
                self.epoch_ms
                    .wrapping_sub(timestamp)
                    .wrapping_mul(1_000_000),
            );
        }
        let fixed = self
            .epoch_ms
            .wrapping_sub(timestamp_ms)
            .wrapping_mul(1_000_000);
        original_ns.wrapping_add(fixed.wrapping_sub(ancestor))
    }
    fn enter_subquery(mut self, original_ns: i64, timestamp_ms: Option<i64>) -> Self {
        self.ancestor_offset_ns = self.ancestor_offset_ns.wrapping_add(original_ns);
        if let Some(timestamp) = timestamp_ms {
            self.ancestor_offset_ns = original_ns;
            // subqueryTimes uses MaxInt64 as its no-timestamp sentinel.
            self.ancestor_timestamp_ms = (timestamp != i64::MAX).then_some(timestamp);
        }
        self
    }
}

#[derive(Clone, Copy)]
struct Scope {
    source: Grid,
    value: Grid,
    tracking: Option<i64>,
    offset_ns: i64,
    child: Option<Result<Grid, GridFault>>,
}

pub(crate) struct Scopes(AccountedVec<Scope>);
impl Scopes {
    pub(crate) fn new(
        expression: &ParsedExpression,
        prepared: &Prepared,
        no_step_interval_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let root = Grid::from(prepared.evaluator_header());
        let mut scopes = AccountedVec::new(account, MemoryKind::Workspace);
        for _ in &expression.arena.nodes {
            control.check()?;
            account.work(1);
            scopes.push(Scope {
                source: root,
                value: root,
                tracking: None,
                offset_ns: 0,
                child: None,
            });
        }
        let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
        tasks.push((expression.root, root, Binding::new(root.start_ms), None));
        while let Some((node, mut grid, binding, tracking)) = tasks.pop() {
            control.check()?;
            account.work(1);
            let source = grid;
            let boundary = prepared.flags().boundaries[node.0];
            if boundary && grid.end_ms >= grid.start_ms {
                grid.end_ms = grid.start_ms;
            }
            let value = &expression.arena[node].expression;
            let offset = |modifiers: &SelectorModifiers| {
                binding.offset(
                    prepared.offset(node, modifiers.original_offset),
                    prepared.timestamp(node, modifiers),
                )
            };
            let scope = &mut scopes[node.0];
            scope.source = source;
            scope.value = grid;
            scope.tracking = tracking;
            match value {
                Expr::Selector(selector) => scope.offset_ns = offset(&selector.modifiers),
                Expr::Subquery {
                    expression: child,
                    range,
                    step,
                    modifiers,
                    ..
                } => {
                    scope.offset_ns = offset(modifiers);
                    let child_grid = grid.subquery(
                        prepared.range(node, *range),
                        prepared.step(node, *step),
                        scope.offset_ns,
                        no_step_interval_ms,
                    );
                    scope.child = Some(child_grid);
                    if let Ok(child_grid) = child_grid {
                        let child_binding = if child_grid.start_ms != grid.start_ms {
                            Binding::new(child_grid.start_ms)
                        } else {
                            binding.enter_subquery(
                                prepared.offset(node, modifiers.original_offset),
                                prepared.timestamp(node, modifiers),
                            )
                        };
                        tasks.push((
                            *child,
                            child_grid,
                            child_binding,
                            Some(child_grid.steps().expect("validated child statistics")),
                        ));
                    }
                    continue;
                }
                _ => (),
            }
            let child_tracking = if boundary { None } else { tracking };
            for child in value.children().rev() {
                control.check()?;
                account.work(1);
                tasks.push((child, grid, binding, child_tracking));
            }
        }
        Ok(Self(scopes))
    }

    pub(crate) fn value(&self, node: NodeId) -> Grid {
        self.0[node.0].value
    }
    pub(crate) fn source(&self, node: NodeId) -> Grid {
        self.0[node.0].source
    }
    pub(crate) fn tracking(&self, node: NodeId) -> Option<i64> {
        self.0[node.0].tracking
    }
    pub(crate) fn offset(&self, node: NodeId) -> i64 {
        self.0[node.0].offset_ns
    }
    pub(crate) fn child(&self, node: NodeId) -> Result<Grid, GridFault> {
        self.0[node.0].child.expect("typed subquery scope")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_step_counts_preserve_go_division_and_wrapping() {
        assert_eq!(quotient(i64::MIN, -1), Ok(i64::MIN));
        assert_eq!(quotient(-3, 2), Ok(-1));
        assert_eq!(quotient(0, 0), Err(GridFault::ZeroInterval));
        assert_eq!(
            Grid {
                start_ms: 0,
                end_ms: i64::MAX,
                interval_ms: 1
            }
            .steps(),
            Ok(i64::MIN)
        );
    }

    #[test]
    fn traversal_positions_survive_wrapped_and_backward_time() {
        // A source loop compares the wrapped timestamp, without treating overflow as EOF.
        let mut clock = Traversal::new(
            Grid {
                start_ms: 0,
                end_ms: 0,
                interval_ms: i64::MIN,
            },
            false,
        );
        assert_eq!(clock.current(), Some((0, 0)));
        clock.advance();
        assert_eq!(clock.current(), Some((i64::MIN, 1)));
        clock.advance();
        assert_eq!(clock.current(), Some((0, 2)));
        let mut shortcut = Traversal::new(
            Grid {
                start_ms: 0,
                end_ms: 0,
                interval_ms: -1,
            },
            true,
        );
        assert_eq!(shortcut.current(), Some((0, 0)));
        shortcut.advance();
        assert_eq!(shortcut.current(), None);
    }
    use crate::{
        accounting::EngineAccounting,
        evaluator_order::preprocess,
        frontend::{ast::ParserOptions, parser},
    };

    fn inspect(
        query: &str,
        header: OutputHeader,
        no_step_interval_ms: i64,
        test: impl FnOnce(&ParsedExpression, &Prepared, &QueryAccounting),
    ) {
        let parsed = parser::parse(
            query.as_bytes(),
            ParserOptions {
                experimental_functions: true,
                ..ParserOptions::default()
            },
            |_| Err::<crate::Strong<dyn crate::frontend::ast::CompiledMatcher>, _>(Vec::<u8>::new()),
        );
        assert!(parsed.errors.is_empty(), "{query}: {:?}", parsed.errors);
        let expression = ParsedExpression {
            source: query.as_bytes().to_vec().into(),
            arena: parsed.arena,
            root: parsed.root.unwrap(),
        };
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let preprocessing =
            preprocess(&expression, MemoryKind::Compiled, &control, &account).unwrap();
        let prepared = Prepared::new(
            &expression,
            &preprocessing,
            header,
            no_step_interval_ms,
            &control,
            &account,
        )
        .unwrap();
        test(&expression, &prepared, &account);
        drop(prepared);
        drop(preprocessing);
        account.snapshot().assert_memory_released();
    }

    fn header(start_ms: i64, end_ms: i64, step_ms: i64) -> OutputHeader {
        OutputHeader {
            value_type: crate::frontend::ast::ValueType::Matrix,
            start_ms,
            end_ms,
            step_ms,
        }
    }

    fn subquery(expression: &ParsedExpression) -> NodeId {
        NodeId(
            expression
                .arena
                .nodes
                .iter()
                .position(|node| matches!(node.expression, Expr::Subquery { .. }))
                .unwrap(),
        )
    }

    #[test]
    fn matrix_arguments_use_typed_positions_and_normalized_nodes() {
        for (query, position, derived) in [
            ("rate(m[3s])", 0, false),
            ("predict_linear(m[3s], 1)", 0, false),
            ("quantile_over_time(0.5, m[3s])", 1, false),
            ("quantile_over_time(0.5, (m)[3s:1s])", 1, true),
            ("predict_linear((m)[3s:1s], 1)", 0, true),
        ] {
            inspect(
                query,
                header(4500, 4500, 0),
                60_000,
                |expression, prepared, account| {
                    let (actual, argument) = crate::evaluator::matrix_argument(
                        expression,
                        prepared,
                        expression.root,
                        &Control::default(),
                        account,
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(actual, position);
                    assert_eq!(
                        matches!(expression.arena[argument].expression, Expr::Subquery { .. }),
                        derived
                    );
                    assert!(matches!(
                        expression.arena[argument].expression,
                        Expr::Matrix { .. } | Expr::Subquery { .. }
                    ));
                },
            );
        }
    }

    #[test]
    fn source_grid_uses_aligned_parent_end_and_strict_lower_boundary() {
        inspect(
            "m[3s:1s]",
            header(500, 5500, 2000),
            60_000,
            |expr, prepared, _| {
                assert_eq!(
                    prepared.scopes.as_ref().unwrap().child(subquery(expr)),
                    Ok(Grid {
                        start_ms: -2000,
                        end_ms: 4500,
                        interval_ms: 1000,
                    })
                );
            },
        );
        let parent = Grid {
            start_ms: -1000,
            end_ms: -1000,
            interval_ms: 0,
        };
        assert_eq!(
            parent.subquery(1_000_000_000, 1_000_000_000, 0, 60_000),
            Ok(Grid {
                start_ms: -1000,
                end_ms: -1000,
                interval_ms: 1000,
            })
        );
    }

    #[test]
    fn same_start_keeps_ancestor_binding_and_changed_start_rebinds() {
        for (time, child_start, offset) in
            [(1000, 1000, 1_500_000_000), (1500, 2000, 2_000_000_000)]
        {
            inspect(
                "(m @0)[1s:1s] offset -500ms",
                header(time, time, 0),
                60_000,
                |expr, prepared, _| {
                    let scopes = prepared.scopes.as_ref().unwrap();
                    let child_grid = scopes.child(subquery(expr)).unwrap();
                    assert_eq!(child_grid.start_ms, child_start);
                    let (node, selector) = expr
                        .arena
                        .nodes
                        .iter()
                        .enumerate()
                        .find_map(|(index, node)| match &node.expression {
                            Expr::Selector(selector) => Some((NodeId(index), selector)),
                            _ => None,
                        })
                        .unwrap();
                    assert_eq!(scopes.value(node).start_ms, child_start);
                    assert_eq!(scopes.offset(node), offset);
                    assert_eq!(
                        prepared.adjusted_offset(node, &selector.modifiers, child_start),
                        offset
                    );
                },
            );
        }
    }

    #[test]
    fn invariant_child_keeps_uncollapsed_grid_and_original_context() {
        inspect(
            "(abs(m @start()) + vector(step()))[3s:1s]",
            header(500, 5500, 2000),
            60_000,
            |expr, prepared, _| {
                let node = subquery(expr);
                let Expr::Subquery {
                    expression: child, ..
                } = expr.arena[node].expression
                else {
                    unreachable!()
                };
                let scopes = prepared.scopes.as_ref().unwrap();
                assert_eq!(scopes.child(node).unwrap().end_ms, 4500);
                assert_eq!(scopes.value(child).start_ms, -2000);
                assert_eq!(scopes.value(child).end_ms, -2000);
                assert_eq!(prepared.context.scalar("start"), Some(0.5));
                assert_eq!(prepared.context.scalar("end"), Some(5.5));
                assert_eq!(prepared.context.scalar("step"), Some(2.0));
                assert_eq!(prepared.context.scalar("range"), Some(5.0));
            },
        );
    }

    #[test]
    fn dynamic_duration_uses_request_context_and_rounded_zero_step_uses_existing_default() {
        inspect(
            "m[range()/2:step()/2] offset (step()/4)",
            header(500, 5500, 2000),
            700,
            |expr, prepared, _| {
                assert_eq!(
                    prepared.scopes.as_ref().unwrap().child(subquery(expr)),
                    Ok(Grid {
                        start_ms: -2000,
                        end_ms: 4000,
                        interval_ms: 1000,
                    })
                );
            },
        );
        inspect(
            "m[3s:(1ms*0.5)]",
            header(4500, 4500, 0),
            700,
            |expr, prepared, _| {
                assert_eq!(
                    prepared.scopes.as_ref().unwrap().child(subquery(expr)),
                    Ok(Grid {
                        start_ms: 2100,
                        end_ms: 4500,
                        interval_ms: 700,
                    })
                );
            },
        );
    }

    #[test]
    fn geometry_retains_integer_domain_and_defers_zero_interval_fault() {
        let parent = Grid {
            start_ms: i64::MIN,
            end_ms: i64::MIN,
            interval_ms: 0,
        };
        assert_eq!(parent.subquery(0, 1, 0, 700), Err(GridFault::ZeroInterval));
        assert_eq!(
            parent.subquery(0, 0, 0, -1),
            Ok(Grid {
                start_ms: i64::MAX,
                end_ms: i64::MIN,
                interval_ms: -1,
            })
        );
        assert_eq!(
            Binding::new(1)
                .enter_subquery(7, Some(i64::MAX))
                .offset(9, Some(0)),
            1_000_002
        );
        let parent = Grid {
            start_ms: 0,
            end_ms: 0,
            interval_ms: 0,
        };
        assert_eq!(
            parent.subquery(1_000_000_000, 0, 0, -1),
            Err(GridFault::NegativeStepCount)
        );
    }

    #[test]
    fn scope_storage_and_work_do_not_scale_with_history_length() {
        let mut observations = Vec::new();
        for end in [5500, 5_500_000_000] {
            inspect(
                "m[3s:1s]",
                header(500, end, 1000),
                700,
                |_, prepared, account| {
                    assert!(prepared.scopes.is_some());
                    let current = account.snapshot();
                    observations.push((current.memory, current.work));
                },
            );
        }
        assert_eq!(observations[0], observations[1]);
    }
}
