// SPDX-License-Identifier: GPL-3.0-or-later

pub mod lexer;
pub mod literal;
mod quote;
pub mod token;
mod unicode;

/// Half-open byte positions in the original expression.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
