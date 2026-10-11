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
mod classic_mutation;
mod classic_replay;
mod count_values;
mod derived_matrix;
mod derived_range;
mod quantile_aggregate;
mod topk;
mod vector_sort;
#[cfg(feature = "fixtures")]
pub use classic_mutation::fixture as classic_mutation_fixture;
#[cfg(feature = "fixtures")]
pub use classic_replay::fixture as classic_replay_fixture;
pub mod evaluator;
mod evaluator_order;
mod evaluator_roster;
pub mod execution;
#[cfg(any(test, feature = "fixtures"))]
pub mod fixture_temporary_storage;
mod float_functions;
mod float_text;
pub mod frontend;
mod go_sort;
mod go_sort_program;
mod histogram_call;
#[cfg(feature = "fixtures")]
pub use go_sort::fixture as sort_fixture;
#[cfg(feature = "fixtures")]
pub use go_sort::selection_order_fixture;
#[cfg(feature = "fixtures")]
pub use topk::ratio_sampler_fixture;
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
mod histogram_text;
mod histogram_trim;
mod histogram_work;
mod info;
mod instant_select;
mod instant_statistics;
mod label_encoding;
#[cfg(test)]
mod label_encoding_tests;
mod label_functions;
mod label_hash;
mod mad;
mod matrix_heads;
mod matrix_reader;
pub mod numerical;
mod numerical_exponential;
mod numerical_hyperbolic;
mod numerical_math;
mod numerical_trigonometric;
mod query_bounds;
mod query_catalog;
mod query_preparation;
mod query_scopes;
mod range_cache;
mod range_extended;
pub mod range_float;
mod range_numeric;
mod range_quantile_rank;
mod range_rates;
mod range_reduce;
pub mod result;
mod shared;
pub use shared::Strong;
#[cfg(test)]
mod selection_tests;
pub mod storage;
mod storage_observed;
mod temporal_quantile;
mod temporal_quantile_sort;
mod temporary_float_bits;
#[cfg(test)]
mod test_storage;
pub mod value;
mod vector_matching;
