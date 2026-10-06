// SPDX-License-Identifier: GPL-3.0-or-later

pub mod ast;
pub mod bytes;
pub(crate) mod float;
pub mod functions;
pub mod lexer;
pub mod literal;
pub mod number;
pub mod parser;
pub(crate) mod quote;
pub mod regex;
pub mod token;
mod unicode;
pub mod validate;
mod work;

/// Half-open byte positions in the original expression.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
