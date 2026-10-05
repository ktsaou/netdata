// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from pinned Prometheus promql/parser/ast.go; see NOTICE.
use super::{Span, functions::Function, token as token_types};
use crate::accounting::{AccountedVec, QueryAccounting};
use std::sync::Arc;
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
    pub name: Vec<u8>,
    pub value: Vec<u8>,
    pub compiled: Option<Arc<dyn CompiledMatcher>>,
}

pub trait CompiledMatcher: std::fmt::Debug + Send + Sync {
    fn matches(&self, value: &[u8]) -> bool {
        self.matches_in(value, None)
    }
    fn matches_in(&self, value: &[u8], accounting: Option<&QueryAccounting>) -> bool;
}

impl Matcher {
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
    pub labels: Option<Vec<Vec<u8>>>,
    pub on: bool,
    pub include: Option<Vec<Vec<u8>>>,
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
    pub name: Vec<u8>,
    pub modifiers: SelectorModifiers,
    pub matchers: Vec<Option<Matcher>>,
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
    String(Vec<u8>),
    Selector(VectorSelector),
    Aggregate {
        operator: TokenKind,
        expression: Option<NodeId>,
        parameter: Option<NodeId>,
        grouping: Option<Vec<Vec<u8>>>,
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
        arguments: Vec<NodeId>,
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

#[derive(Clone, Debug)]
pub struct Node {
    pub expression: Expr,
    pub span: Span,
    pub value_type: ValueType,
}

#[derive(Debug, Default)]
pub struct Arena {
    pub nodes: Vec<Node>,
}

impl Arena {
    /// Evaluation uses the inner argument without mutating captured source nodes.
    pub(crate) fn unparenthesized(&self, mut id: NodeId) -> NodeId {
        while let Expr::Paren(child) = &self[id].expression {
            id = *child;
        }
        id
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
    pub source: Box<[u8]>,
    pub arena: Arena,
    pub root: NodeId,
}
