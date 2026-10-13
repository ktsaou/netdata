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
fn temporary(
    value: accounting::TemporaryStorageObservations,
) -> types::TemporaryStorageObservations {
    types::TemporaryStorageObservations {
        extent: memory(value.extent),
        stores: reference(value.stores),
        read_calls: value.read_calls,
        write_calls: value.write_calls,
        read_bytes: value.read_bytes,
        written_bytes: value.written_bytes,
        counter_overflowed: u32::from(value.counter_overflowed),
        reserved: 0,
    }
}
pub(crate) fn query(value: &accounting::QueryObservations) -> types::QueryObservations {
    types::QueryObservations {
        statistics: types::Statistics {
            timings: types::StatisticsTimings {
                eval_total_time: duration(value.statistics.timings.eval_total_time),
                result_sort_time: duration(value.statistics.timings.result_sort_time),
                query_preparation_time: duration(value.statistics.timings.query_preparation_time),
                inner_eval_time: duration(value.statistics.timings.inner_eval_time),
                exec_queue_time: duration(value.statistics.timings.exec_queue_time),
                exec_total_time: duration(value.statistics.timings.exec_total_time),
            },
            samples: types::StatisticsSamples {
                total_queryable_samples: value.statistics.samples.total_queryable_samples,
                samples_read: value.statistics.samples.samples_read,
                peak_samples: value.statistics.samples.peak_samples,
            },
        },
        memory: value.memory.map(memory),
        cancellation_references: references(value.cancellation_references),
        temporary_storage: temporary(value.temporary_storage),
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
        temporary_storage: temporary(value.temporary_storage),
        active_executions: value.active_executions,
        peak_executions: value.peak_executions,
        completed_executions: value.completed_executions,
        elapsed: duration(value.elapsed),
        counter_overflowed: u32::from(value.counter_overflowed),
        reserved: 0,
    }
}
