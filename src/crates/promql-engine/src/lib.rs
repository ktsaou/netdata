// SPDX-License-Identifier: GPL-3.0-or-later
//! Standalone typed PromQL expression core, qualified against a pinned reference.
//! Includes bounded typed storage, partial paged evaluation and independently owned output.
//! Complete expression qualification and the C boundary remain in progress.

#[cfg(panic = "abort")]
compile_error!("PromQL requires panic=unwind for containment at its C boundary");

mod absence;
pub mod accounting;
mod aggregation;
pub mod annotations;
mod binary_labels;
mod binary_value;
pub mod c_abi;
mod calendar;
mod classic_replay;
#[cfg(feature = "fixtures")]
pub use classic_replay::fixture as classic_replay_fixture;
pub mod evaluator;
mod evaluator_order;
mod evaluator_roster;
pub mod execution;
mod float_functions;
mod float_text;
pub mod frontend;
mod go_sort;
mod histogram_call;
#[cfg(feature = "fixtures")]
pub use go_sort::fixture as sort_fixture;
pub mod histogram;
mod histogram_accumulator;
mod histogram_arithmetic;
mod histogram_bounds;
mod histogram_bounds_table;
mod histogram_classic;
mod histogram_counter;
mod histogram_equality;
mod histogram_functions;
mod histogram_iterator;
mod histogram_layout;
mod histogram_stats;
mod histogram_trim;
mod histogram_work;
mod instant_statistics;
mod label_functions;
mod label_hash;
pub mod numerical;
mod numerical_exponential;
mod numerical_hyperbolic;
mod numerical_math;
mod numerical_trigonometric;
mod query_bounds;
mod query_catalog;
mod query_preparation;
pub mod range_float;
mod range_numeric;
mod range_rates;
mod range_reduce;
pub mod result;
mod shared;
pub use shared::Strong;
#[cfg(test)]
mod selection_tests;
pub mod storage;
mod storage_observed;
#[cfg(test)]
mod test_storage;
pub mod value;
mod vector_matching;
