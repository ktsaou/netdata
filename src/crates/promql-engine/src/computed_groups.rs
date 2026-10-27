// SPDX-License-Identifier: GPL-3.0-or-later
//! Completed grouped values in provider backing; no metric-by-time RAM matrix.
use crate::{
    accounting::{AccountedBox, AllocationContext, MemoryKind, QueryAccounting},
    query_scopes::Grid,
    statistics::OwnedSamples,
    storage::{
        Control, OwnedTemporary, QueryProvider, StorageError, TemporaryFloatBits,
        TemporaryFloatBitsRequest,
    },
    vector_matching::ReplayKey,
};

const WORDS: usize = 1024;
const CELL_WORDS: u64 = 4;
const PRESENT: u64 = 2;
const ABSENT: u64 = 1;
const DROP_NAME: u64 = 4;
const FINAL: u64 = 8;
const UNREAD: u64 = 16;

#[cfg(test)]
thread_local! {
    static FUTURE_DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OBSERVATION: std::cell::Cell<Option<(bool, [usize; 23])>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) struct Observation(Option<(bool, [usize; 23])>);
#[cfg(test)]
impl Drop for Observation {
    fn drop(&mut self) {
        OBSERVATION.set(self.0);
    }
}
#[cfg(test)]
pub(crate) fn disable() -> Observation {
    Observation(OBSERVATION.replace(Some((true, [0; 23]))))
}
#[cfg(test)]
pub(crate) fn observe<R>(disabled: bool, operation: impl FnOnce() -> R) -> (R, [usize; 23]) {
    let _restore = Observation(OBSERVATION.replace(Some((disabled, [0; 23]))));
    let result = operation();
    (result, OBSERVATION.get().unwrap().1)
}

#[cfg(test)]
fn note(index: usize, selector: bool) {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        counts[index] += 1;
        if selector {
            counts[index + 4] += 1;
        }
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[cfg(test)]
fn transfer(write: bool, selector: bool, words: usize) {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        let index = 8 + usize::from(selector) * 2 + usize::from(write);
        counts[index] += 1;
        counts[index + 4] += words * 8;
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[cfg(test)]
pub(crate) fn window_pass(completed: bool) {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        counts[16] += 1;
        counts[17] += usize::from(completed);
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[cfg(test)]
pub(crate) fn materialized_window() {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        counts[18] += 1;
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[cfg(test)]
pub(crate) fn seeded_lookup() {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        counts[19] += 1;
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[cfg(test)]
pub(crate) struct FutureObservation(bool);
#[cfg(test)]
impl Drop for FutureObservation {
    fn drop(&mut self) {
        FUTURE_DISABLED.set(self.0);
    }
}
#[cfg(test)]
pub(crate) fn disable_future() -> FutureObservation {
    FutureObservation(FUTURE_DISABLED.replace(true))
}
#[cfg(test)]
pub(crate) fn future_disabled() -> bool {
    FUTURE_DISABLED.get()
}
#[cfg(test)]
fn future_event(index: usize) {
    if let Some((disabled, mut counts)) = OBSERVATION.get() {
        counts[index] += 1;
        OBSERVATION.set(Some((disabled, counts)));
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Point {
    pub(crate) bits: u64,
    pub(crate) time_ms: i64,
    pub(crate) start_time_ms: i64,
    pub(crate) drop_name: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        evaluator::EvaluationOptions,
        storage::{ProviderResources, SelectionRequest, TimeBounds},
        storage_observed::ObservedQuery,
    };

    struct Query<const BACKING: bool>;
    impl<const BACKING: bool> QueryProvider for Query<BACKING> {
        type Selection<'a> = crate::test_storage::FixtureSelection<'a>;
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn select(
            &self,
            _: SelectionRequest<'_>,
            _: &Control,
        ) -> Result<Self::Selection<'_>, StorageError> {
            unreachable!("computed backing test does not read raw data")
        }
        fn open_temporary_float_bits(
            &self,
            request: TemporaryFloatBitsRequest<'_>,
            control: &Control,
        ) -> Result<Option<OwnedTemporary<'_>>, StorageError> {
            if BACKING {
                crate::fixture_temporary_storage::open_store(request, control)
            } else {
                Ok(None)
            }
        }
    }
    fn key(grid: Grid) -> ReplayKey {
        ReplayKey::new(
            EvaluationOptions {
                time_ms: grid.start_ms,
                start_ms: grid.start_ms,
                end_ms: grid.end_ms,
                step_ms: grid.interval_ms,
                lookback_ms: 17,
                start_timestamps: true,
                delayed_name_removal: true,
            },
            0,
            17_000_000,
            TimeBounds {
                start_ms: -99,
                end_ms: 1000,
            },
        )
    }
    #[test]
    fn completed_group_pages_preserve_bits_absence_scope_and_fresh_final_generation() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = ObservedQuery::new(Query::<true>, &account);
        let control = Control::default();
        let grid = Grid {
            start_ms: -200,
            end_ms: 200,
            interval_ms: 1,
        };
        let key = key(grid);
        let mut store = Store::create(&query, 3, grid, key, &control, &account)
            .unwrap()
            .unwrap();
        let patterns = [
            0x8000_0000_0000_0000,
            0x7ff8_0000_0000_0001,
            0x7ff0_0000_0000_0002,
        ];
        for step in 0..401 {
            let time = grid.start_ms + step;
            let at = key.at(time, step);
            if step % 3 != 0 {
                let ordinal = step as usize % 3;
                store
                    .write(
                        at,
                        time,
                        ordinal,
                        Some(Point {
                            bits: patterns[ordinal],
                            time_ms: time - 7,
                            start_time_ms: time - 99,
                            drop_name: ordinal == 1,
                        }),
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            assert!(matches!(
                store.read(at, time, 0, &control, &account).unwrap(),
                Cell::Unknown
            ));
        }
        store.publish(&control, &account).unwrap();
        for step in (0..401).rev() {
            let time = grid.start_ms + step;
            for (ordinal, pattern) in patterns.iter().enumerate() {
                let cell = store
                    .read(key.at(time, step), time, ordinal, &control, &account)
                    .unwrap();
                if step % 3 != 0 && step as usize % 3 == ordinal {
                    let Cell::Present(point) = cell else {
                        panic!("present point")
                    };
                    assert_eq!(point.bits, *pattern);
                    assert_eq!((point.time_ms, point.start_time_ms), (time - 7, time - 99));
                    assert_eq!(point.drop_name, ordinal == 1);
                } else {
                    assert!(matches!(cell, Cell::Absent));
                }
            }
        }
        assert!(matches!(
            store
                .read(key.at(0, 199), 0, 1, &control, &account)
                .unwrap(),
            Cell::Unknown
        ));
        store.start_final();
        let at = key.at(1, 201);
        assert!(matches!(
            store.read(at, 1, 1, &control, &account).unwrap(),
            Cell::Unknown
        ));
        store.write(at, 1, 1, None, &control, &account).unwrap();
        assert!(matches!(
            store.read(at, 1, 1, &control, &account).unwrap(),
            Cell::Absent
        ));
        assert!(matches!(
            store.read(at, 1, 2, &control, &account).unwrap(),
            Cell::Unknown
        ));
        let before = account.snapshot();
        assert_eq!(before.temporary_storage.extent.current, 401 * 3 * 4 * 8);
        assert!(store.spare.is_some());
        assert!(before.memory[MemoryKind::Workspace as usize].peak < 3 * WORDS as u64 * 8);
        control.cancellation.cancel();
        assert!(matches!(
            store.read(at, 1, 1, &control, &account),
            Err(StorageError::Interrupted(_))
        ));
        drop(store);
        drop(query);
        account.snapshot().assert_memory_released();
        assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
    }

    #[test]
    fn missing_computed_backing_is_optional_and_extent_overflow_is_explicit() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let query = Query::<false>;
        let control = Control::default();
        let grid = Grid {
            start_ms: 0,
            end_ms: 1000,
            interval_ms: 1,
        };
        assert!(
            Store::create(&query, 4, grid, key(grid), &control, &account)
                .unwrap()
                .is_none()
        );
        let huge = Grid {
            start_ms: 0,
            end_ms: i64::MAX - 1,
            interval_ms: 1,
        };
        assert!(matches!(
            Store::create(&query, 4, huge, key(huge), &control, &account),
            Err(StorageError::Invalid("computed group extent overflow"))
        ));
        account.snapshot().assert_memory_released();
    }

    #[derive(Default)]
    struct Transfers {
        writes: std::cell::Cell<usize>,
        reads: std::cell::Cell<usize>,
        fail_write: Option<usize>,
        fail_read: bool,
        cancel_write: bool,
        cancel_read: bool,
        fail_next_read: std::cell::Cell<bool>,
        fail_next_write: std::cell::Cell<bool>,
        cancel_next_read: std::cell::Cell<bool>,
        cancel_next_write: std::cell::Cell<bool>,
    }
    struct DirtyBacking<'a> {
        words: crate::accounting::AccountedVec<u64>,
        initialized: usize,
        transfers: &'a Transfers,
    }
    impl TemporaryFloatBits for DirtyBacking<'_> {
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn read_into(
            &mut self,
            offset: u64,
            values: &mut [u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            self.transfers.reads.set(self.transfers.reads.get() + 1);
            if self.transfers.fail_read || self.transfers.fail_next_read.replace(false) {
                return Err(StorageError::Invalid("injected backing read"));
            }
            let start = offset as usize;
            assert!(
                start + values.len() <= self.initialized,
                "unwritten backing was read"
            );
            values.copy_from_slice(&self.words[start..start + values.len()]);
            if self.transfers.cancel_read || self.transfers.cancel_next_read.replace(false) {
                control.cancellation.cancel();
            }
            Ok(())
        }
        fn write_at(
            &mut self,
            offset: u64,
            values: &[u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            let call = self.transfers.writes.get() + 1;
            self.transfers.writes.set(call);
            if self.transfers.fail_write == Some(call)
                || self.transfers.fail_next_write.replace(false)
            {
                return Err(StorageError::Invalid("injected backing write"));
            }
            let start = offset as usize;
            assert!(start <= self.initialized, "initialization skipped a gap");
            self.words[start..start + values.len()].copy_from_slice(values);
            self.initialized = self.initialized.max(start + values.len());
            if self.transfers.cancel_write || self.transfers.cancel_next_write.replace(false) {
                control.cancellation.cancel();
            }
            Ok(())
        }
    }
    impl QueryProvider for Transfers {
        type Selection<'a> = crate::test_storage::FixtureSelection<'a>;
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn select(
            &self,
            _: SelectionRequest<'_>,
            _: &Control,
        ) -> Result<Self::Selection<'_>, StorageError> {
            unreachable!()
        }
        fn open_temporary_float_bits(
            &self,
            request: TemporaryFloatBitsRequest<'_>,
            _: &Control,
        ) -> Result<Option<OwnedTemporary<'_>>, StorageError> {
            let mut words =
                crate::accounting::AccountedVec::new(request.accounting, MemoryKind::ProviderHeld);
            words.resize(request.value_count as usize, PRESENT);
            Ok(Some(OwnedTemporary::new(
                DirtyBacking {
                    words,
                    initialized: 0,
                    transfers: self,
                },
                request.accounting,
            )))
        }
    }

    #[test]
    fn future_selector_cells_admit_once_and_overlaps_preserve_completed_values() {
        let account = EngineAccounting::default().query();
        let query = Transfers::default();
        let control = Control::default();
        let grid = Grid {
            start_ms: 0,
            end_ms: 400,
            interval_ms: 1,
        };
        let key = key(grid);
        let mut store = Store::create(&query, 3, grid, key, &control, &account)
            .unwrap()
            .unwrap();
        store.selector_points();
        let point = Point {
            bits: (-0.0_f64).to_bits(),
            time_ms: 0,
            start_time_ms: -7,
            drop_name: false,
        };
        store
            .write_future_selector(key, 0, 0, Some(point), &control, &account)
            .unwrap();
        store.publish(&control, &account).unwrap();
        assert!(matches!(
            store.read(key, 0, 0, &control, &account).unwrap(),
            Cell::Unknown
        ));
        store.start_final();
        for time in [1, 150, 300] {
            store
                .write_future_selector(key.at(time, time), time, 0, Some(point), &control, &account)
                .unwrap();
        }
        store
            .write_future_selector(key, 0, 0, None, &control, &account)
            .unwrap();
        store.publish(&control, &account).unwrap();
        for time in [300, 150, 1] {
            assert!(matches!(
                store
                    .read(key.at(time, time), time, 0, &control, &account)
                    .unwrap(),
                Cell::Present(_)
            ));
        }
        assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
        for (expected, time) in [(1, 1), (2, 150), (3, 300)] {
            store
                .write_future_selector(
                    key.at(time, time),
                    time,
                    0,
                    Some(Point { bits: 99, ..point }),
                    &control,
                    &account,
                )
                .unwrap();
            let Cell::Present(actual) = store
                .read_selector(key.at(time, time), time, 0, &control, &account)
                .unwrap()
            else {
                panic!("prefetched cell");
            };
            assert_eq!(actual.bits, point.bits);
            assert_eq!(account.snapshot().statistics.samples.samples_read, expected);
            store
                .write_future_selector(key.at(time, time), time, 0, Some(point), &control, &account)
                .unwrap();
            store
                .read_selector(key.at(time, time), time, 0, &control, &account)
                .unwrap();
            assert_eq!(account.snapshot().statistics.samples.samples_read, expected);
        }
        store.publish(&control, &account).unwrap();
        for time in [1, 300, 150] {
            store
                .read_selector(key.at(time, time), time, 0, &control, &account)
                .unwrap();
        }
        assert!(matches!(
            store.read_selector(key, 0, 0, &control, &account).unwrap(),
            Cell::Absent
        ));
        assert_eq!(account.snapshot().statistics.samples.samples_read, 3);
        drop(store);
        account.snapshot().assert_memory_released();
        assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
    }

    #[test]
    fn future_records_preserve_dirty_residents_and_bound_strided_transfers() {
        for initialized in [false, true] {
            let account = EngineAccounting::default().query();
            let query = Transfers::default();
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 3,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, 512, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            store.selector_points();
            if initialized {
                store
                    .write(key.at(3, 3), 3, 511, None, &control, &account)
                    .unwrap();
            }
            store.publish(&control, &account).unwrap();
            store.start_final();
            for ordinal in [0, 256] {
                store
                    .write(
                        key,
                        0,
                        ordinal,
                        Some(Point {
                            bits: ordinal as u64 + 99,
                            time_ms: -3,
                            start_time_ms: -7,
                            drop_name: true,
                        }),
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            assert!(store.page.dirty && store.spare.as_ref().unwrap().dirty);
            let residents = (store.page.base, store.spare.as_ref().unwrap().base);
            let before = account.snapshot();
            let points = [
                (
                    2,
                    0,
                    Some(Point {
                        bits: (-0.0_f64).to_bits(),
                        time_ms: -2,
                        start_time_ms: -9,
                        drop_name: true,
                    }),
                ),
                (2, 1, None),
                (
                    3,
                    511,
                    Some(Point {
                        bits: 0x7ff8_0000_0000_1234,
                        time_ms: -1,
                        start_time_ms: -8,
                        drop_name: false,
                    }),
                ),
            ];
            for &(time, ordinal, point) in &points {
                store
                    .write_future_selector(
                        key.at(time, time),
                        time,
                        ordinal,
                        point,
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            if initialized {
                let after = account.snapshot();
                assert_eq!(
                    after.temporary_storage.read_bytes - before.temporary_storage.read_bytes,
                    3 * CELL_WORDS * 8
                );
                assert_eq!(
                    after.temporary_storage.written_bytes - before.temporary_storage.written_bytes,
                    3 * CELL_WORDS * 8
                );
                assert_eq!(
                    (store.page.base, store.spare.as_ref().unwrap().base),
                    residents
                );
                assert!(store.page.dirty && store.spare.as_ref().unwrap().dirty);
                assert_eq!(
                    after.statistics.samples.peak_samples,
                    before.statistics.samples.peak_samples + 1
                );
            }
            let before = account.snapshot().temporary_storage.written_bytes;
            for &(time, ordinal, _) in &points {
                store
                    .write_future_selector(
                        key.at(time, time),
                        time,
                        ordinal,
                        Some(Point {
                            bits: 17,
                            time_ms: 17,
                            start_time_ms: 17,
                            drop_name: false,
                        }),
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            if initialized {
                assert_eq!(
                    account.snapshot().temporary_storage.written_bytes,
                    before,
                    "overlaps must not replace known or absent records"
                );
            }
            store.publish(&control, &account).unwrap();
            assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
            for _ in 0..2 {
                for &(time, ordinal, expected) in points.iter().rev() {
                    let actual = store
                        .read_selector(key.at(time, time), time, ordinal, &control, &account)
                        .unwrap();
                    match (actual, expected) {
                        (Cell::Absent, None) => (),
                        (Cell::Present(actual), Some(expected)) => assert_eq!(
                            (
                                actual.bits,
                                actual.time_ms,
                                actual.start_time_ms,
                                actual.drop_name
                            ),
                            (
                                expected.bits,
                                expected.time_ms,
                                expected.start_time_ms,
                                expected.drop_name
                            )
                        ),
                        _ => panic!("future record changed"),
                    }
                }
                for ordinal in [256, 0] {
                    let Cell::Present(point) = store
                        .read_selector(key, 0, ordinal, &control, &account)
                        .unwrap()
                    else {
                        panic!("dirty resident lost");
                    };
                    assert_eq!(
                        (
                            point.bits,
                            point.time_ms,
                            point.start_time_ms,
                            point.drop_name
                        ),
                        (ordinal as u64 + 99, -3, -7, true)
                    );
                }
                assert_eq!(account.snapshot().statistics.samples.samples_read, 2);
            }
            drop(store);
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }

    #[test]
    fn future_record_transfers_preserve_errors_cancellation_and_transient_credits() {
        for fault in 0..6 {
            let account = EngineAccounting::default().query();
            let query = Transfers::default();
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 3,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, 512, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            store.selector_points();
            store
                .write(key.at(3, 3), 3, 511, None, &control, &account)
                .unwrap();
            store.publish(&control, &account).unwrap();
            store.start_final();
            let point = Some(Point {
                bits: 42,
                time_ms: -2,
                start_time_ms: -7,
                drop_name: false,
            });
            if fault == 5 {
                store
                    .write_future_selector(key.at(2, 2), 2, 0, point, &control, &account)
                    .unwrap();
            }
            match fault {
                1 => query.fail_next_read.set(true),
                2 | 5 => query.cancel_next_read.set(true),
                3 => query.fail_next_write.set(true),
                4 => query.cancel_next_write.set(true),
                _ => (),
            }
            let result = store.write_future_selector(key.at(2, 2), 2, 0, point, &control, &account);
            assert!(matches!(
                (fault, &result),
                (0, Ok(()))
                    | (1, Err(StorageError::Invalid("injected backing read")))
                    | (2 | 4 | 5, Err(StorageError::Interrupted(_)))
                    | (3, Err(StorageError::Invalid("injected backing write")))
            ));
            let observed = account.snapshot();
            assert_eq!(observed.statistics.samples.samples_read, 0);
            assert_eq!(
                observed.statistics.samples.peak_samples,
                u64::from(fault == 0 || fault >= 3)
            );
            assert!(store.page.base.is_none() && store.spare.is_none());
            drop(store);
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }

    #[test]
    fn group_window_read_policy_counts_distinct_pages_with_exact_scope() {
        for groups in [1, 16, 63, 64, 255, 256, 257, 512] {
            let account = EngineAccounting::default().query();
            let query = Query::<true>;
            let control = Control::default();
            let grid = Grid {
                start_ms: -20,
                end_ms: 20,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, groups, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            for first in 0..4_i64 {
                for last in first..=40 {
                    for ordinal in [0, groups - 1] {
                        let pages: std::collections::BTreeSet<_> = (first..=last)
                            .map(|index| {
                                (index as u64 * groups as u64 + ordinal as u64) * CELL_WORDS
                                    / WORDS as u64
                            })
                            .collect();
                        assert_eq!(
                            store.window_pages(
                                (key.at(first - 20, first), first - 20),
                                (key.at(last - 20, last), last - 20),
                                ordinal
                            ),
                            Some(pages.len() as u64)
                        );
                    }
                }
            }
            assert_eq!(
                store.window_cell_reads((key, -20), (key.at(20, 40), 20), 0),
                groups >= 64,
                "dense windows batch values per page"
            );
            assert!(!store.window_cell_reads((key, -20), (key.at(20, 39), 20), 0));
            assert!(!store.window_cell_reads((key, -20), (key.at(21, 41), 21), 0));
            store.selector_points();
            assert!(!store.window_cell_reads((key, -20), (key.at(20, 40), 20), 0));
            drop(store);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn completed_group_windows_reuse_two_pages_in_both_generations() {
        for final_phase in [false, true] {
            let account = EngineAccounting::default().query();
            let query = Transfers::default();
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 31,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, 16, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            if final_phase {
                store.publish(&control, &account).unwrap();
                store.start_final();
            }
            for time in 0..32 {
                for ordinal in 0..16 {
                    let point = (ordinal != 15).then_some(Point {
                        bits: time as u64 * 16 + ordinal as u64,
                        time_ms: time - 3,
                        start_time_ms: -7,
                        drop_name: true,
                    });
                    store
                        .write(key.at(time, time), time, ordinal, point, &control, &account)
                        .unwrap();
                }
            }
            assert!(store.page.dirty && store.spare.as_ref().unwrap().dirty);
            store.publish(&control, &account).unwrap();
            let before = account.snapshot().temporary_storage;
            assert!(!store.window_cell_reads((key.at(14, 14), 14), (key.at(17, 17), 17), 0));
            for _ in 0..16 {
                for time in [14, 17, 15, 16] {
                    for ordinal in [0, 15] {
                        let (cell, lease) = store
                            .read_window(
                                key.at(time, time),
                                time,
                                ordinal,
                                false,
                                &control,
                                &account,
                            )
                            .unwrap();
                        assert!(lease.is_none());
                        match cell {
                            Cell::Present(point) => assert_eq!(
                                (
                                    point.bits,
                                    point.time_ms,
                                    point.start_time_ms,
                                    point.drop_name
                                ),
                                (time as u64 * 16, time - 3, -7, true)
                            ),
                            Cell::Absent => assert_eq!(ordinal, 15),
                            Cell::Unknown => panic!("completed group missing"),
                        }
                    }
                }
            }
            assert_eq!(
                account.snapshot().temporary_storage.read_bytes,
                before.read_bytes
            );
            assert_eq!(
                account.snapshot().temporary_storage.written_bytes,
                before.written_bytes
            );
            drop(store);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn wide_group_windows_hold_small_records_through_consumption_and_failures() {
        for final_phase in [false, true] {
            for fault in 0..3 {
                let account = EngineAccounting::default().query();
                let query = Transfers::default();
                let control = Control::default();
                let grid = Grid {
                    start_ms: 0,
                    end_ms: 3,
                    interval_ms: 1,
                };
                let key = key(grid);
                let mut store = Store::create(&query, 256, grid, key, &control, &account)
                    .unwrap()
                    .unwrap();
                if final_phase {
                    store.publish(&control, &account).unwrap();
                    store.start_final();
                }
                for time in 0..4 {
                    store
                        .write(
                            key.at(time, time),
                            time,
                            0,
                            Some(Point {
                                bits: (-0.0_f64).to_bits() + time as u64,
                                time_ms: time - 9,
                                start_time_ms: -17,
                                drop_name: true,
                            }),
                            &control,
                            &account,
                        )
                        .unwrap();
                }
                store.publish(&control, &account).unwrap();
                // Leave a resident dirty while a different completed page is consumed.
                store
                    .write(key.at(3, 3), 3, 1, None, &control, &account)
                    .unwrap();
                let residents = (store.page.base, store.spare.as_ref().unwrap().base);
                let before = account.snapshot();
                assert!(store.window_cell_reads((key, 0), (key.at(3, 3), 3), 0));
                query.fail_next_read.set(fault == 1);
                query.cancel_next_read.set(fault == 2);
                let result = store.read_window(key, 0, 0, true, &control, &account);
                if fault == 0 {
                    let (Cell::Present(point), Some(lease)) = result.unwrap() else {
                        panic!("completed nonresident record");
                    };
                    assert_eq!(
                        (
                            point.bits,
                            point.time_ms,
                            point.start_time_ms,
                            point.drop_name
                        ),
                        ((-0.0_f64).to_bits(), -9, -17, true)
                    );
                    assert_eq!(lease.units(), 1);
                    assert_eq!(
                        account.snapshot().statistics.samples.peak_samples,
                        before.statistics.samples.peak_samples + 1
                    );
                    control.cancellation.cancel();
                    assert!(control.check().is_err());
                    drop(lease);
                } else {
                    assert!(matches!(
                        (fault, result),
                        (1, Err(StorageError::Invalid("injected backing read")))
                            | (2, Err(StorageError::Interrupted(_)))
                    ));
                }
                assert_eq!(
                    (store.page.base, store.spare.as_ref().unwrap().base),
                    residents
                );
                assert!(store.page.dirty);
                assert_eq!(
                    account.snapshot().temporary_storage.read_bytes
                        - before.temporary_storage.read_bytes,
                    if fault == 1 { 0 } else { CELL_WORDS * 8 }
                );
                assert_eq!(
                    account.snapshot().temporary_storage.written_bytes,
                    before.temporary_storage.written_bytes
                );
                assert_eq!(account.snapshot().statistics.samples.samples_read, 0);
                drop(store);
                account.snapshot().assert_memory_released();
                assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
            }
        }
    }

    #[test]
    fn preparation_selector_misses_read_only_the_completed_cell() {
        for far in [false, true] {
            for fault in 0..3 {
                let account = EngineAccounting::default().query();
                let query = Transfers {
                    fail_read: fault == 1,
                    cancel_read: fault == 2,
                    ..Transfers::default()
                };
                let control = Control::default();
                let grid = Grid {
                    start_ms: 0,
                    end_ms: 400,
                    interval_ms: 1,
                };
                let key = key(grid);
                let mut store = Store::create(&query, 3, grid, key, &control, &account)
                    .unwrap()
                    .unwrap();
                store.selector_points();
                let point = Point {
                    bits: 0xfff8_0000_0000_1234,
                    time_ms: -3,
                    start_time_ms: -7,
                    drop_name: true,
                };
                store
                    .write(key, 0, 0, Some(point), &control, &account)
                    .unwrap();
                store
                    .write(key.at(1, 1), 1, 0, None, &control, &account)
                    .unwrap();
                if far {
                    store
                        .write(key.at(400, 400), 400, 2, Some(point), &control, &account)
                        .unwrap();
                }
                store.publish(&control, &account).unwrap();
                let base = store.page.base;
                let before = account.snapshot();
                let result = store.read(key, 0, 0, &control, &account);
                if far && fault != 0 {
                    assert!(matches!(
                        (&result, fault),
                        (Err(StorageError::Invalid("injected backing read")), 1)
                            | (Err(StorageError::Interrupted(_)), 2)
                    ));
                    if fault == 2 {
                        assert_eq!(account.snapshot().statistics.samples.peak_samples, 2);
                        assert_eq!(
                            account.snapshot().temporary_storage.read_bytes
                                - before.temporary_storage.read_bytes,
                            CELL_WORDS * 8
                        );
                    }
                } else {
                    let Cell::Present(actual) = result.unwrap() else {
                        panic!("completed Float cell")
                    };
                    assert_eq!(
                        (
                            actual.bits,
                            actual.time_ms,
                            actual.start_time_ms,
                            actual.drop_name
                        ),
                        (
                            point.bits,
                            point.time_ms,
                            point.start_time_ms,
                            point.drop_name
                        )
                    );
                    assert_eq!(
                        account.snapshot().temporary_storage.read_bytes
                            - before.temporary_storage.read_bytes,
                        if far { CELL_WORDS * 8 } else { 0 }
                    );
                    assert!(matches!(
                        store.read(key.at(1, 1), 1, 0, &control, &account).unwrap(),
                        Cell::Absent
                    ));
                    assert!(matches!(
                        store.read(key.at(2, 2), 2, 0, &control, &account).unwrap(),
                        Cell::Unknown
                    ));
                    let reads = query.reads.get();
                    let last = store
                        .read(key.at(400, 400), 400, 2, &control, &account)
                        .unwrap();
                    if far {
                        assert!(matches!(last, Cell::Present(_)));
                    } else {
                        assert!(matches!(last, Cell::Unknown));
                    }
                    assert_eq!(query.reads.get(), reads);
                }
                assert_eq!(store.page.base, base);
                assert!(store.spare.is_none());
                drop(store);
                account.snapshot().assert_memory_released();
                assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
            }
        }
    }

    #[test]
    fn final_selector_page_pair_preserves_previous_clock_and_bounded_transfers() {
        for groups in [1, 255, 256, 257, 513] {
            let account = EngineAccounting::default().query();
            let query = ObservedQuery::new(Transfers::default(), &account);
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 4,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, groups, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            store.selector_points();
            for ordinal in 0..groups {
                for time in 0..5 {
                    store
                        .write(
                            key.at(time, time),
                            time,
                            ordinal,
                            Some(Point {
                                bits: (-0.0_f64).to_bits(),
                                time_ms: -3,
                                start_time_ms: -7,
                                drop_name: true,
                            }),
                            &control,
                            &account,
                        )
                        .unwrap();
                }
            }
            store.publish(&control, &account).unwrap();
            assert!(store.spare.is_none());
            store.start_final();
            let before = account.snapshot().temporary_storage;
            for time in 0..5 {
                for ordinal in 0..groups {
                    assert!(matches!(
                        store
                            .read(key.at(time, time), time, ordinal, &control, &account)
                            .unwrap(),
                        Cell::Unknown
                    ));
                    if time != 0 {
                        let Cell::Present(previous) = store
                            .read(
                                key.at(time - 1, time - 1),
                                time - 1,
                                ordinal,
                                &control,
                                &account,
                            )
                            .unwrap()
                        else {
                            panic!("previous final clock");
                        };
                        assert_eq!(
                            previous.bits,
                            ((time - 1) as u64 * groups as u64 + ordinal as u64)
                        );
                        assert_eq!((previous.time_ms, previous.start_time_ms), (time - 2, -17));
                        assert!(!previous.drop_name);
                    }
                    store
                        .write(
                            key.at(time, time),
                            time,
                            ordinal,
                            Some(Point {
                                bits: time as u64 * groups as u64 + ordinal as u64,
                                time_ms: time - 1,
                                start_time_ms: -17,
                                drop_name: false,
                            }),
                            &control,
                            &account,
                        )
                        .unwrap();
                }
            }
            store.publish(&control, &account).unwrap();
            let after = account.snapshot();
            let extent_pages = store.words.div_ceil(WORDS as u64);
            assert!(
                after.temporary_storage.read_bytes - before.read_bytes
                    <= 20 * extent_pages * WORDS as u64 * 8
            );
            assert!(after.memory[MemoryKind::Workspace as usize].peak < 24 * 1024);
            assert_eq!(
                store.spare.is_some(),
                groups * 5 > WORDS / CELL_WORDS as usize
            );
            for time in (0..5).rev() {
                for ordinal in (0..groups).rev() {
                    let Cell::Present(point) = store
                        .read(key.at(time, time), time, ordinal, &control, &account)
                        .unwrap()
                    else {
                        panic!("retained final cell")
                    };
                    assert_eq!(point.bits, time as u64 * groups as u64 + ordinal as u64);
                }
            }
            drop(store);
            drop(query);
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }

    #[test]
    fn two_dirty_selector_pages_preserve_initialization_and_failure_cleanup() {
        for failure in 0..6 {
            let account = EngineAccounting::default().query();
            let query = ObservedQuery::new(
                Transfers {
                    fail_write: (1..=4).contains(&failure).then_some(failure),
                    cancel_write: failure == 5,
                    ..Transfers::default()
                },
                &account,
            );
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 400,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, 3, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            store.selector_points();
            store.publish(&control, &account).unwrap();
            store.start_final();
            for time in [90, 300] {
                store
                    .write(
                        key.at(time, time),
                        time,
                        0,
                        Some(Point {
                            bits: time as u64,
                            time_ms: time,
                            start_time_ms: 0,
                            drop_name: false,
                        }),
                        &control,
                        &account,
                    )
                    .unwrap();
            }
            assert!(store.page.dirty && store.spare.as_ref().unwrap().dirty);
            let result = store.publish(&control, &account);
            if failure != 0 {
                assert!(result.is_err());
            } else {
                result.unwrap();
                // Evict both residents, then reconstruct each from the initialized backing.
                assert!(matches!(
                    store.read(key, 0, 0, &control, &account).unwrap(),
                    Cell::Unknown
                ));
                assert!(matches!(
                    store
                        .read(key.at(200, 200), 200, 0, &control, &account)
                        .unwrap(),
                    Cell::Unknown
                ));
                for time in [300, 90] {
                    let Cell::Present(point) = store
                        .read(key.at(time, time), time, 0, &control, &account)
                        .unwrap()
                    else {
                        panic!("dirty page persisted")
                    };
                    assert_eq!(point.bits, time as u64);
                }
            }
            drop(store);
            drop(query);
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }

    #[test]
    fn selector_pages_initialize_sparse_backing_and_preserve_failure_cleanup() {
        for failure in 0..5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let transfers = Transfers {
                fail_write: match failure {
                    1 => Some(1),
                    2 => Some(5),
                    _ => None,
                },
                fail_read: failure == 3,
                cancel_write: failure == 4,
                ..Transfers::default()
            };
            let query = ObservedQuery::new(transfers, &account);
            let control = Control::default();
            let grid = Grid {
                start_ms: 0,
                end_ms: 400,
                interval_ms: 1,
            };
            let key = key(grid);
            let mut store = Store::create(&query, 3, grid, key, &control, &account)
                .unwrap()
                .unwrap();
            store.selector_points();
            let far = key.at(300, 300);
            store
                .write(
                    far,
                    300,
                    2,
                    Some(Point {
                        bits: (-0.0_f64).to_bits(),
                        time_ms: 297,
                        start_time_ms: 7,
                        drop_name: false,
                    }),
                    &control,
                    &account,
                )
                .unwrap();
            let published = store.publish(&control, &account);
            if matches!(failure, 1 | 2 | 4) {
                assert!(published.is_err());
                assert!(!store.complete);
            } else {
                published.unwrap();
                let first = store.read(key, 0, 0, &control, &account);
                if failure == 3 {
                    assert!(matches!(
                        first,
                        Err(StorageError::Invalid("injected backing read"))
                    ));
                } else {
                    assert!(matches!(first.unwrap(), Cell::Unknown));
                    let Cell::Present(point) = store.read(far, 300, 2, &control, &account).unwrap()
                    else {
                        panic!("retained selector point")
                    };
                    assert_eq!(
                        (point.bits, point.time_ms, point.start_time_ms),
                        ((-0.0_f64).to_bits(), 297, 7)
                    );
                    store.start_final();
                    assert!(matches!(
                        store.read(far, 300, 2, &control, &account).unwrap(),
                        Cell::Unknown
                    ));
                    store.write(far, 300, 2, None, &control, &account).unwrap();
                    assert!(matches!(
                        store.read(far, 300, 2, &control, &account).unwrap(),
                        Cell::Absent
                    ));
                    assert!(matches!(
                        store
                            .read(key.at(299, 299), 299, 2, &control, &account)
                            .unwrap(),
                        Cell::Unknown
                    ));
                    assert!(
                        account.snapshot().temporary_storage.written_bytes
                            >= 3 * 401 * CELL_WORDS * 8
                    );
                }
            }
            drop(store);
            drop(query);
            account.snapshot().assert_memory_released();
            assert_eq!(account.snapshot().temporary_storage.extent.current, 0);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Cell {
    Unknown,
    Absent,
    Present(Point),
}

pub(crate) struct Store<'q> {
    backing: OwnedTemporary<'q>,
    page: AccountedBox<Page>,
    spare: Option<AccountedBox<Page>>,
    words: u64,
    initialized_words: u64,
    groups: u64,
    grid: Grid,
    key: ReplayKey,
    complete: bool,
    final_phase: bool,
    explicit_absence: bool,
    series_major: bool,
}

struct Page {
    buffer: [u64; WORDS],
    base: Option<u64>,
    len: usize,
    dirty: bool,
    samples: OwnedSamples,
}

impl Page {
    fn new(account: &QueryAccounting) -> AccountedBox<Self> {
        AccountedBox::new(
            Self {
                buffer: [0; WORDS],
                base: None,
                len: 0,
                dirty: false,
                samples: OwnedSamples::new(account, 0),
            },
            &AllocationContext::new(account, MemoryKind::Workspace),
        )
    }

    fn clear(&mut self) {
        self.base = None;
        self.dirty = false;
        self.samples.resize(0);
    }
}

impl<'q> Store<'q> {
    pub(crate) fn create<Q: QueryProvider>(
        query: &'q Q,
        groups: usize,
        grid: Grid,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<Self>, StorageError> {
        #[cfg(test)]
        if OBSERVATION.get().is_some_and(|(disabled, _)| disabled) {
            return Ok(None);
        }
        if groups == 0 || grid.start_ms > grid.end_ms || !grid.forward_without_wrap() {
            return Ok(None);
        }
        let Some(width) = grid.end_ms.checked_sub(grid.start_ms) else {
            return Ok(None);
        };
        let steps = (width / grid.interval_ms) as u64 + 1;
        // A single cell has no cross-group/time replay to amortize a backing owner.
        // Existing singleton-fold reuse handles that shape without opening another store.
        if groups == 1 && steps == 1 {
            return Ok(None);
        }
        let groups = u64::try_from(groups)
            .map_err(|_| StorageError::Invalid("computed group count overflow"))?;
        let words = groups
            .checked_mul(steps)
            .and_then(|cells| cells.checked_mul(CELL_WORDS))
            .ok_or(StorageError::Invalid("computed group extent overflow"))?;
        let request = TemporaryFloatBitsRequest {
            value_count: words,
            accounting: account,
        };
        request.extent_bytes()?;
        control.check()?;
        account.provider_call();
        let Some(backing) = query.open_temporary_float_bits(request, control)? else {
            return Ok(None);
        };
        let store = Self {
            backing,
            page: Page::new(account),
            spare: None,
            words,
            initialized_words: 0,
            groups,
            grid,
            key,
            complete: false,
            final_phase: false,
            explicit_absence: false,
            series_major: false,
        };
        Ok(Some(store))
    }

    fn offset(&self, key: ReplayKey, time_ms: i64, ordinal: usize) -> Option<u64> {
        let delta = time_ms.checked_sub(self.grid.start_ms)?;
        if delta < 0 || time_ms > self.grid.end_ms || delta % self.grid.interval_ms != 0 {
            return None;
        }
        let step = delta / self.grid.interval_ms;
        if self.key.at(time_ms, step) != key || ordinal as u64 >= self.groups {
            return None;
        }
        let cell = if self.series_major {
            ordinal as u64 * (self.words / CELL_WORDS / self.groups) + step as u64
        } else {
            step as u64 * self.groups + ordinal as u64
        };
        Some(cell * CELL_WORDS)
    }

    pub(crate) fn selector_points(&mut self) {
        self.explicit_absence = true;
        self.series_major = true;
    }

    pub(crate) fn building(&self, key: ReplayKey, time_ms: i64, ordinal: usize) -> bool {
        !self.complete && !self.final_phase && self.offset(key, time_ms, ordinal).is_some()
    }

    pub(crate) fn prepared(&self, key: ReplayKey, time_ms: i64) -> bool {
        self.complete && !self.final_phase && self.offset(key, time_ms, 0).is_some()
    }

    pub(crate) fn records_final(&self, key: ReplayKey, time_ms: i64, ordinal: usize) -> bool {
        self.complete && self.final_phase && self.offset(key, time_ms, ordinal).is_some()
    }

    fn flush(&mut self, control: &Control, account: &QueryAccounting) -> Result<(), StorageError> {
        if self.page.dirty
            && self.spare.as_ref().is_some_and(|spare| {
                spare.dirty
                    && spare.base >= Some(self.initialized_words)
                    && spare.base < self.page.base
            })
        {
            // Preserve a lower dirty page before initializing the gap to the current page.
            std::mem::swap(&mut self.page, self.spare.as_mut().unwrap());
            let result = self.flush_current(control, account);
            std::mem::swap(&mut self.page, self.spare.as_mut().unwrap());
            result?;
        }
        self.flush_current(control, account)
    }

    fn flush_current(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        if self.page.dirty {
            let base = self.page.base.expect("dirty computed page");
            // Track initialized backing explicitly; providers need not zero new extents.
            // Fill only gaps before a reached page, never the unused remainder of the query grid.
            while self.initialized_words < base {
                let len = (base - self.initialized_words).min(WORDS as u64) as usize;
                crate::temporary_float_bits::write(
                    &mut self.backing,
                    self.initialized_words,
                    &[0; WORDS][..len],
                    control,
                    account,
                )?;
                #[cfg(test)]
                transfer(true, self.explicit_absence, len);
                account.work(len as u64);
                self.initialized_words += len as u64;
            }
            crate::temporary_float_bits::write(
                &mut self.backing,
                base,
                &self.page.buffer[..self.page.len],
                control,
                account,
            )?;
            #[cfg(test)]
            transfer(true, self.explicit_absence, self.page.len);
            self.initialized_words = self.initialized_words.max(base + self.page.len as u64);
            self.page.dirty = false;
        }
        Ok(())
    }

    fn page(
        &mut self,
        offset: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, StorageError> {
        control.check()?;
        account.work(1);
        let base = offset - offset % WORDS as u64;
        if self.page.base != Some(base) {
            if (self.final_phase || !self.explicit_absence) && self.page.base.is_some() {
                if self.spare.is_none() {
                    control.check()?;
                    self.spare = Some(Page::new(account));
                }
                std::mem::swap(&mut self.page, self.spare.as_mut().unwrap());
                if self.page.base == Some(base) {
                    return Ok((offset - base) as usize);
                }
            }
            self.flush(control, account)?;
            self.page.base = None;
            self.page.samples.resize(0);
            self.page.len = (self.words - base).min(WORDS as u64) as usize;
            let initialized = self
                .initialized_words
                .saturating_sub(base)
                .min(self.page.len as u64) as usize;
            if initialized != 0 {
                account.provider_call();
                account.temporary_transfer(false, None);
                self.backing
                    .read_into(base, &mut self.page.buffer[..initialized], control)
                    .map_err(|error| error.accounted(account))?;
                account.temporary_transfer(false, Some(initialized as u64 * 8));
                #[cfg(test)]
                transfer(false, self.explicit_absence, initialized);
            }
            let len = self.page.len;
            self.page.buffer[initialized..len].fill(0);
            account.work((self.page.len - initialized) as u64);
            self.page.base = Some(base);
            self.observe_samples(account);
            control.check()?;
        }
        Ok((offset - base) as usize)
    }

    fn observe_samples(&mut self, account: &QueryAccounting) {
        let tag = if self.final_phase { FINAL } else { 0 };
        let present = self.page.buffer[..self.page.len]
            .chunks_exact(CELL_WORDS as usize)
            .filter(|cell| cell[0] & !(DROP_NAME | UNREAD) == (tag | PRESENT))
            .count();
        account.work(self.page.len as u64 / CELL_WORDS);
        self.page.samples.resize(present as u64);
    }

    pub(crate) fn read(
        &mut self,
        key: ReplayKey,
        time_ms: i64,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Cell, StorageError> {
        if !self.complete {
            return Ok(Cell::Unknown);
        }
        let Some(offset) = self.offset(key, time_ms, ordinal) else {
            return Ok(Cell::Unknown);
        };
        if !self.final_phase
            && self.explicit_absence
            && self.series_major
            && self.page.base != Some(offset - offset % WORDS as u64)
        {
            let (cell, _sample) = self.read_cell(offset, control, account)?;
            return Ok(self.decode(&cell));
        }
        let index = self.page(offset, control, account)?;
        Ok(self.decode(&self.page.buffer[index..index + CELL_WORDS as usize]))
    }

    pub(crate) fn window_cell_reads(
        &self,
        first: (ReplayKey, i64),
        last: (ReplayKey, i64),
        ordinal: usize,
    ) -> bool {
        // Dense columns amortize a page callback across many window values.
        self.groups >= WORDS as u64 / (CELL_WORDS * 4)
            && self
                .window_pages(first, last, ordinal)
                .is_some_and(|pages| pages > 2)
    }

    fn window_pages(
        &self,
        first: (ReplayKey, i64),
        last: (ReplayKey, i64),
        ordinal: usize,
    ) -> Option<u64> {
        if self.explicit_absence || self.series_major || last.1 < first.1 {
            return None;
        }
        let (Some(first_offset), Some(last_offset)) = (
            self.offset(first.0, first.1, ordinal),
            self.offset(last.0, last.1, ordinal),
        ) else {
            return None;
        };
        let steps =
            (i128::from(last.1) - i128::from(first.1)) / i128::from(self.grid.interval_ms) + 1;
        let pages = last_offset / WORDS as u64 - first_offset / WORDS as u64 + 1;
        Some(steps.min(i128::from(pages)) as u64)
    }

    pub(crate) fn read_window(
        &mut self,
        key: ReplayKey,
        time_ms: i64,
        ordinal: usize,
        direct: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(Cell, Option<OwnedSamples>), StorageError> {
        if direct
            && self.complete
            && !self.explicit_absence
            && let Some(offset) = self.offset(key, time_ms, ordinal)
        {
            let base = offset - offset % WORDS as u64;
            if self.page.base != Some(base)
                && !self
                    .spare
                    .as_ref()
                    .is_some_and(|page| page.base == Some(base))
            {
                let (cell, samples) = self.read_cell(offset, control, account)?;
                return Ok((self.decode(&cell), Some(samples)));
            }
        }
        Ok((self.read(key, time_ms, ordinal, control, account)?, None))
    }

    fn read_cell(
        &mut self,
        offset: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<([u64; CELL_WORDS as usize], OwnedSamples), StorageError> {
        control.check()?;
        account.work(1);
        let mut cell = [0; CELL_WORDS as usize];
        if offset + CELL_WORDS <= self.initialized_words {
            account.provider_call();
            account.temporary_transfer(false, None);
            self.backing
                .read_into(offset, &mut cell, control)
                .map_err(|error| error.accounted(account))?;
            account.temporary_transfer(false, Some(CELL_WORDS * 8));
            #[cfg(test)]
            transfer(false, self.explicit_absence, CELL_WORDS as usize);
        }
        account.work(CELL_WORDS);
        let tag = if self.final_phase { FINAL } else { 0 };
        let sample = OwnedSamples::new(
            account,
            u64::from(cell[0] & !(DROP_NAME | UNREAD) == (tag | PRESENT)),
        );
        control.check()?;
        Ok((cell, sample))
    }

    fn encode(&self, point: Option<Point>, pending: bool) -> [u64; CELL_WORDS as usize] {
        let tag = if self.final_phase { FINAL } else { 0 } | if pending { UNREAD } else { 0 };
        match point {
            Some(point) => [
                tag | PRESENT | if point.drop_name { DROP_NAME } else { 0 },
                point.bits,
                point.time_ms as u64,
                point.start_time_ms as u64,
            ],
            None => [tag | ABSENT, 0, 0, 0],
        }
    }

    pub(crate) fn read_selector(
        &mut self,
        key: ReplayKey,
        time_ms: i64,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Cell, StorageError> {
        let cell = self.read(key, time_ms, ordinal, control, account)?;
        if self.final_phase && matches!(cell, Cell::Present(_) | Cell::Absent) {
            let offset = self
                .offset(key, time_ms, ordinal)
                .expect("validated selector cell");
            let index = (offset - self.page.base.expect("resident final cell")) as usize;
            if self.page.buffer[index] & UNREAD != 0 {
                self.page.buffer[index] &= !UNREAD;
                self.page.dirty = true;
                account.samples_read(u64::from(matches!(cell, Cell::Present(_))));
                #[cfg(test)]
                future_event(20);
            }
        }
        Ok(cell)
    }

    fn decode(&self, cell: &[u64]) -> Cell {
        let flags = cell[0];
        let tag = if self.final_phase { FINAL } else { 0 };
        if flags & !(DROP_NAME | UNREAD) == (tag | PRESENT) {
            #[cfg(test)]
            note(usize::from(self.final_phase), self.explicit_absence);
            Cell::Present(Point {
                bits: cell[1],
                time_ms: cell[2] as i64,
                start_time_ms: cell[3] as i64,
                drop_name: flags & DROP_NAME != 0,
            })
        } else if flags & !UNREAD == (tag | ABSENT)
            || !self.final_phase && !self.explicit_absence && flags == 0
        {
            #[cfg(test)]
            note(usize::from(self.final_phase), self.explicit_absence);
            Cell::Absent
        } else {
            #[cfg(test)]
            note(2, self.explicit_absence);
            Cell::Unknown
        }
    }

    pub(crate) fn write(
        &mut self,
        key: ReplayKey,
        time_ms: i64,
        ordinal: usize,
        point: Option<Point>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        let offset = self
            .offset(key, time_ms, ordinal)
            .ok_or(StorageError::Invalid(
                "computed group producer scope changed",
            ))?;
        let index = self.page(offset, control, account)?;
        let tag = if self.final_phase { FINAL } else { 0 };
        let was_present = self.page.buffer[index] & !(DROP_NAME | UNREAD) == (tag | PRESENT);
        let cell = self.encode(point, false);
        self.page.buffer[index..index + CELL_WORDS as usize].copy_from_slice(&cell);
        self.page.dirty = true;
        #[cfg(test)]
        if self.final_phase {
            note(3, self.explicit_absence);
        }
        let units = self.page.samples.units() - u64::from(was_present) + u64::from(point.is_some());
        self.page.samples.resize(units);
        Ok(())
    }

    pub(crate) fn write_future_selector(
        &mut self,
        key: ReplayKey,
        time_ms: i64,
        ordinal: usize,
        point: Option<Point>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        if !self.explicit_absence || !self.records_final(key, time_ms, ordinal) {
            return Ok(());
        }
        let offset = self
            .offset(key, time_ms, ordinal)
            .expect("validated final cell");
        let base = offset - offset % WORDS as u64;
        if offset + CELL_WORDS <= self.initialized_words
            && self.page.base != Some(base)
            && !self
                .spare
                .as_ref()
                .is_some_and(|page| page.base == Some(base))
        {
            // Strided future writes must not evict the pages serving current clocks.
            let (old, mut sample) = self.read_cell(offset, control, account)?;
            if !matches!(self.decode(&old), Cell::Unknown) {
                #[cfg(test)]
                future_event(21);
                return Ok(());
            }
            let cell = self.encode(point, true);
            sample.resize(u64::from(point.is_some()));
            account.work(CELL_WORDS);
            crate::temporary_float_bits::write(&mut self.backing, offset, &cell, control, account)?;
            #[cfg(test)]
            {
                transfer(true, true, CELL_WORDS as usize);
                note(3, true);
                future_event(22);
            }
            return Ok(());
        }
        if !matches!(
            self.read(key, time_ms, ordinal, control, account)?,
            Cell::Unknown
        ) {
            #[cfg(test)]
            future_event(21);
            return Ok(());
        }
        self.write(key, time_ms, ordinal, point, control, account)?;
        #[cfg(test)]
        future_event(22);
        {
            let offset = self
                .offset(key, time_ms, ordinal)
                .expect("published selector cell");
            let index = (offset - self.page.base.expect("resident final cell")) as usize;
            self.page.buffer[index] |= UNREAD;
        }
        Ok(())
    }

    pub(crate) fn publish(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), StorageError> {
        self.flush(control, account)?;
        if let Some(spare) = self.spare.as_mut() {
            std::mem::swap(&mut self.page, spare);
            self.flush(control, account)?;
        }
        control.check()?;
        self.complete = true;
        Ok(())
    }

    pub(crate) fn start_final(&mut self) {
        // Preparation tags remain invalid even when final selectors change their cell layout.
        self.final_phase = true;
        self.series_major = false;
        self.page.clear();
        if let Some(spare) = self.spare.as_mut() {
            spare.clear();
        }
    }
}
