// SPDX-License-Identifier: GPL-3.0-or-later
//! Standalone typed PromQL expression core, qualified against a pinned reference.
//! Storage, evaluation and the C boundary are implemented separately from this frontend.

#[cfg(panic = "abort")]
compile_error!("PromQL requires panic=unwind for containment at its C boundary");

pub mod frontend;
pub mod histogram;
mod histogram_arithmetic;
mod histogram_bounds;
mod histogram_bounds_table;
mod histogram_counter;
mod histogram_equality;
mod histogram_iterator;
mod histogram_layout;
mod histogram_trim;
pub mod numerical;
mod numerical_math;
pub mod value;
