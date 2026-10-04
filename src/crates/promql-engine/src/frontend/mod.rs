// SPDX-License-Identifier: GPL-3.0-or-later

pub mod ast;
pub mod functions;
pub mod lexer;
pub mod literal;
pub mod number;
pub mod parser;
mod quote;
pub mod token;
mod unicode;
pub mod validate;

/// Half-open byte positions in the original expression.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
