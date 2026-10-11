// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from pinned Prometheus promql/parser/ast.go; see NOTICE.
use super::{Span, functions::Function, token as token_types};
use crate::{
    Strong,
    accounting::{AccountedVec, AllocationContext, QueryAccounting},
};
use token_types::TokenKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType {
    Scalar,
    Vector,
    Matrix,
    String,
    Invalid,
}

impl ValueType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Vector => "vector",
            Self::Matrix => "matrix",
            Self::String => "string",
            Self::Invalid => "none",
        }
    }

    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Scalar => "scalar",
            Self::Vector => "instant vector",
            Self::Matrix => "range vector",
            Self::String => "string",
            Self::Invalid => "none",
        }
    }
}

/// Arena references keep traversal and destruction independent of nesting depth.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct NodeId(pub usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MatchType {
    Equal,
    NotEqual,
    Regex,
    NotRegex,
}

#[derive(Clone, Debug)]
pub struct Matcher {
    pub kind: MatchType,
    pub name: AccountedVec<u8>,
    pub value: AccountedVec<u8>,
    pub compiled: Option<Strong<dyn CompiledMatcher>>,
}

pub trait CompiledMatcher: std::fmt::Debug + Send + Sync {
    fn matches(&self, value: &[u8]) -> bool {
        self.matches_in(value, None)
    }
    fn matches_in(&self, value: &[u8], accounting: Option<&QueryAccounting>) -> bool;
    fn matches_checked(
        &self,
        value: &[u8],
        accounting: Option<&QueryAccounting>,
        control: &crate::storage::Control,
    ) -> Result<bool, crate::storage::Interrupted> {
        control.check()?;
        let result = self.matches_in(value, accounting);
        control.check()?;
        Ok(result)
    }
}

impl Matcher {
    pub(crate) fn matches_with(
        &self,
        value: &[u8],
        accounting: Option<&QueryAccounting>,
        control: Option<&crate::storage::Control>,
    ) -> Result<bool, crate::storage::Interrupted> {
        let work = super::work::Work {
            context: None,
            account: accounting,
            control,
        };
        work.step(0)?;
        match self.kind {
            MatchType::Equal => work.equal(&self.value, value),
            MatchType::NotEqual => Ok(!work.equal(&self.value, value)?),
            MatchType::Regex | MatchType::NotRegex => {
                let compiled = self.compiled.as_ref().expect("validated regexp matcher");
                let result = match control {
                    Some(control) => compiled.matches_checked(value, accounting, control)?,
                    None => compiled.matches_in(value, accounting),
                };
                Ok(if self.kind == MatchType::NotRegex {
                    !result
                } else {
                    result
                })
            }
        }
    }
    pub(crate) fn copy_for(
        &self,
        account: Option<&QueryAccounting>,
        kind: crate::accounting::MemoryKind,
        control: &crate::storage::Control,
    ) -> Result<Self, crate::storage::Interrupted> {
        let context = account.map(|account| AllocationContext::new(account, kind));
        let work = super::work::Work {
            context: context.as_ref(),
            account,
            control: Some(control),
        };
        Ok(Self {
            kind: self.kind,
            name: work.copy(&self.name)?,
            value: work.copy(&self.value)?,
            compiled: self.compiled.clone(),
        })
    }

    pub fn matches(&self, value: &[u8]) -> bool {
        self.matches_in(value, None)
    }
    pub fn matches_in(&self, value: &[u8], accounting: Option<&QueryAccounting>) -> bool {
        match self.kind {
            MatchType::Equal => self.value == value,
            MatchType::NotEqual => self.value != value,
            MatchType::Regex => self
                .compiled
                .as_ref()
                .expect("validated regexp matcher")
                .matches_in(value, accounting),
            MatchType::NotRegex => !self
                .compiled
                .as_ref()
                .expect("validated regexp matcher")
                .matches_in(value, accounting),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(u8)]
pub enum Cardinality {
    #[default]
    OneToOne,
    ManyToOne,
    OneToMany,
    ManyToMany,
}

#[derive(Clone, Debug, Default)]
pub struct VectorMatching {
    pub cardinality: Cardinality,
    pub labels: Option<AccountedVec<AccountedVec<u8>>>,
    pub on: bool,
    pub include: Option<AccountedVec<AccountedVec<u8>>>,
    pub fill_left: Option<f64>,
    pub fill_right: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct SelectorModifiers {
    pub original_offset: i64,
    pub original_offset_expression: Option<NodeId>,
    pub offset: i64,
    pub timestamp: Option<i64>,
    pub start_or_end: Option<TokenKind>,
}

#[derive(Clone, Debug, Default)]
pub struct VectorSelector {
    pub name: AccountedVec<u8>,
    pub modifiers: SelectorModifiers,
    pub matchers: AccountedVec<Option<Matcher>>,
    pub skip_histogram_buckets: bool,
    pub bypass_empty_matcher_check: bool,
    pub anchored: bool,
    pub smoothed: bool,
}

#[derive(Clone, Debug)]
pub enum Expr {
    Number {
        value: f64,
        duration: bool,
    },
    String(AccountedVec<u8>),
    Selector(VectorSelector),
    Aggregate {
        operator: TokenKind,
        expression: Option<NodeId>,
        parameter: Option<NodeId>,
        grouping: Option<AccountedVec<AccountedVec<u8>>>,
        without: bool,
    },
    Binary {
        operator: TokenKind,
        left: NodeId,
        right: NodeId,
        matching: Option<VectorMatching>,
        return_bool: bool,
    },
    Call {
        function: Option<&'static Function>,
        arguments: AccountedVec<NodeId>,
    },
    Matrix {
        selector: NodeId,
        range: i64,
        range_expression: Option<NodeId>,
    },
    Subquery {
        expression: NodeId,
        range: i64,
        range_expression: Option<NodeId>,
        modifiers: SelectorModifiers,
        step: i64,
        step_expression: Option<NodeId>,
    },
    Paren(NodeId),
    Unary {
        operator: TokenKind,
        expression: NodeId,
    },
    Duration {
        operator: TokenKind,
        left: Option<NodeId>,
        right: Option<NodeId>,
        wrapped: bool,
        start: usize,
        end: usize,
    },
}

impl Expr {
    /// Expression children after source duration preprocessing; modifier durations are separate.
    pub(crate) fn children(&self) -> impl DoubleEndedIterator<Item = NodeId> + '_ {
        let (fixed, slice): ([Option<NodeId>; 2], &[NodeId]) = match self {
            Self::Paren(child)
            | Self::Unary {
                expression: child, ..
            }
            | Self::Subquery {
                expression: child, ..
            }
            | Self::Matrix {
                selector: child, ..
            } => ([Some(*child), None], &[]),
            Self::Aggregate {
                expression,
                parameter,
                ..
            } => ([*expression, *parameter], &[]),
            Self::Binary { left, right, .. } => ([Some(*left), Some(*right)], &[]),
            Self::Call { arguments, .. } => ([None, None], arguments),
            _ => ([None, None], &[]),
        };
        fixed.into_iter().flatten().chain(slice.iter().copied())
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub expression: Expr,
    pub span: Span,
    pub value_type: ValueType,
}

#[derive(Debug, Default)]
pub struct Arena {
    pub nodes: AccountedVec<Node>,
}

impl Arena {
    pub(crate) fn new_in(context: Option<&AllocationContext>) -> Self {
        Self {
            nodes: AccountedVec::new_in(context),
        }
    }
    /// Evaluation uses the inner argument without mutating captured source nodes.
    pub(crate) fn unparenthesized(
        &self,
        mut id: NodeId,
        control: &crate::storage::Control,
        account: &QueryAccounting,
    ) -> Result<NodeId, crate::storage::Interrupted> {
        control.check()?;
        while let Expr::Paren(child) = &self[id].expression {
            control.check()?;
            account.work(1);
            id = *child;
        }
        control.check()?;
        Ok(id)
    }

    pub fn push(&mut self, expression: Expr, span: Span) -> NodeId {
        let value_type = match &expression {
            Expr::Number { .. } | Expr::Duration { .. } => ValueType::Scalar,
            Expr::String(_) => ValueType::String,
            Expr::Selector(_) | Expr::Aggregate { .. } => ValueType::Vector,
            Expr::Matrix { .. } | Expr::Subquery { .. } => ValueType::Matrix,
            Expr::Call { function, .. } => function.map_or(ValueType::Invalid, |f| f.return_type),
            Expr::Paren(child)
            | Expr::Unary {
                expression: child, ..
            } => self[*child].value_type,
            Expr::Binary { left, right, .. } => {
                if self[*left].value_type == ValueType::Scalar
                    && self[*right].value_type == ValueType::Scalar
                {
                    ValueType::Scalar
                } else {
                    ValueType::Vector
                }
            }
        };
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            expression,
            span,
            value_type,
        });
        id
    }
}

impl std::ops::Index<NodeId> for Arena {
    type Output = Node;
    fn index(&self, id: NodeId) -> &Self::Output {
        &self.nodes[id.0]
    }
}

impl std::ops::IndexMut<NodeId> for Arena {
    fn index_mut(&mut self, id: NodeId) -> &mut Self::Output {
        &mut self.nodes[id.0]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParserOptions {
    pub experimental_functions: bool,
    pub binop_fill_modifiers: bool,
    pub extended_range_selectors: bool,
    pub duration_expressions: bool,
}

impl Default for ParserOptions {
    fn default() -> Self {
        Self {
            experimental_functions: false,
            binop_fill_modifiers: false,
            extended_range_selectors: false,
            duration_expressions: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseError {
    pub span: Span,
    pub message: AccountedVec<u8>,
}

#[derive(Debug)]
pub struct ParsedExpression {
    pub source: AccountedVec<u8>,
    pub arena: Arena,
    pub root: NodeId,
}

#[cfg(test)]
mod checked_matcher_tests {
    use super::*;
    use crate::storage::{Cancellation, Control, Interrupted};
    #[derive(Debug)]
    struct Legacy(Cancellation);
    impl CompiledMatcher for Legacy {
        fn matches_in(&self, _value: &[u8], _account: Option<&QueryAccounting>) -> bool {
            self.0.cancel();
            false
        }
    }
    #[test]
    fn legacy_callback_interruption_is_not_negated_into_a_match() {
        let control = Control::default();
        let allocation: Box<crate::shared::Allocation<dyn CompiledMatcher>> = Box::new(
            crate::shared::Allocation::new(Legacy(control.cancellation.clone())),
        );
        let compiled = Strong::finish(allocation, None);
        let matcher = Matcher {
            kind: MatchType::NotRegex,
            name: b"x".to_vec().into(),
            value: b"a".to_vec().into(),
            compiled: Some(compiled),
        };
        assert_eq!(
            matcher.matches_with(b"value", None, Some(&control)),
            Err(Interrupted::Cancelled)
        );
    }
}
