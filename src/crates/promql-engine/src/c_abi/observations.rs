// SPDX-License-Identifier: GPL-3.0-or-later
//! Wire snapshots keep duration and counter representations independent of Rust layouts.
use super::types;
use crate::accounting;

pub(crate) fn duration(value: std::time::Duration) -> types::Duration {
    types::Duration {
        seconds: value.as_secs(),
        nanoseconds: value.subsec_nanos(),
        reserved: 0,
    }
}
fn memory(value: accounting::MemoryObservation) -> types::MemoryObservation {
    types::MemoryObservation {
        current: value.current,
        peak: value.peak,
        acquired: value.acquired,
        released: value.released,
        overflowed: u32::from(value.overflowed),
        reserved: 0,
    }
}
fn reference(value: accounting::ReferenceObservation) -> types::ReferenceObservation {
    types::ReferenceObservation {
        current: value.current,
        peak: value.peak,
        acquired: value.acquired,
        released: value.released,
        overflowed: u32::from(value.overflowed),
        reserved: 0,
    }
}
fn references(value: accounting::CancellationReferences) -> types::CancellationReferences {
    types::CancellationReferences {
        engine_owned: reference(value.engine_owned),
        external: reference(value.external),
    }
}
pub(crate) fn query(value: &accounting::QueryObservations) -> types::QueryObservations {
    types::QueryObservations {
        memory: value.memory.map(memory),
        cancellation_references: references(value.cancellation_references),
        provider_calls: value.provider_calls,
        pages: value.pages,
        samples_fetched: value.samples_fetched,
        work: value.work,
        output_samples: value.output_samples,
        elapsed: duration(value.elapsed),
        counter_overflowed: u32::from(value.counter_overflowed),
        reserved: 0,
    }
}
pub(crate) fn engine(value: &accounting::EngineObservations) -> types::EngineObservations {
    types::EngineObservations {
        memory: value.memory.map(memory),
        cancellation_references: references(value.cancellation_references),
        active_executions: value.active_executions,
        peak_executions: value.peak_executions,
        completed_executions: value.completed_executions,
        elapsed: duration(value.elapsed),
        counter_overflowed: u32::from(value.counter_overflowed),
        reserved: 0,
    }
}
