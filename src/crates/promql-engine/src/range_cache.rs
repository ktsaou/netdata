// SPDX-License-Identifier: GPL-3.0-or-later
//! Query-owned completed folds and source-certified cursor progress, without raw history.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationKind,
    evaluator::{EvaluationError, EvaluationOptions},
    frontend::ast::NodeId,
    matrix_reader::{Geometry, MatrixWindow, Progress},
    range_float::RateInput,
    range_numeric::{NumericRangeInput, NumericRangeOutput},
    range_rates::RateOutput,
    range_reduce::{FoldResult, RangeFunction},
    result::OwnedValue,
    shared::Strong,
    storage::{Control, Interrupted, TimeBounds},
    vector_matching::ReplayKey,
};
use std::cell::RefCell;

#[derive(Clone, Copy, Eq, PartialEq)]
struct Scope {
    start_ms: i64,
    end_ms: i64,
    step_ms: i64,
    lookback_ms: i64,
    start_timestamps: bool,
    delayed_name_removal: bool,
}
impl From<EvaluationOptions> for Scope {
    fn from(options: EvaluationOptions) -> Self {
        Self {
            start_ms: options.start_ms,
            end_ms: options.end_ms,
            step_ms: options.step_ms,
            lookback_ms: options.lookback_ms,
            start_timestamps: options.start_timestamps,
            delayed_name_removal: options.delayed_name_removal,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum Input {
    Rate {
        start_ms: i64,
        end_ms: i64,
        range_ns: i64,
        start_timestamps: bool,
        range_start_ms: i64,
        range_end_ms: i64,
        data_bounds: Option<[i64; 2]>,
    },
    Fold {
        bounds: Option<[i64; 2]>,
        anchor_start_ms: Option<i64>,
    },
    Numeric {
        bounds: Option<[i64; 2]>,
        seek_ms: i64,
    },
}
fn bounds_key(bounds: Option<TimeBounds>) -> Option<[i64; 2]> {
    bounds.map(|bounds| [bounds.start_ms, bounds.end_ms])
}
impl From<RateInput> for Input {
    fn from(input: RateInput) -> Self {
        Self::Rate {
            start_ms: input.window.start_ms,
            end_ms: input.window.end_ms,
            range_ns: input.window.range_ns,
            start_timestamps: input.window.start_timestamps,
            range_start_ms: input.range_start_ms,
            range_end_ms: input.range_end_ms,
            data_bounds: bounds_key(input.data_bounds),
        }
    }
}
impl From<NumericRangeInput> for Input {
    fn from(input: NumericRangeInput) -> Self {
        Self::Numeric {
            bounds: bounds_key(input.bounds),
            seek_ms: input.seek_ms,
        }
    }
}
impl Input {
    pub(crate) fn fold(bounds: Option<TimeBounds>, anchor_start_ms: Option<i64>) -> Self {
        Self::Fold {
            bounds: bounds_key(bounds),
            anchor_start_ms,
        }
    }
}

pub(crate) struct Request {
    pub(crate) owner: NodeId,
    pub(crate) selector: NodeId,
    pub(crate) ordinal: usize,
    pub(crate) options: EvaluationOptions,
    pub(crate) lookback_ns: i64,
    pub(crate) position: i64,
    pub(crate) function: RangeFunction,
    pub(crate) parameters: [f64; 2],
    pub(crate) range_ns: i64,
    pub(crate) window: MatrixWindow,
    pub(crate) input: Input,
}
#[derive(Clone, Copy, Eq, PartialEq)]
struct Key {
    scope: Scope,
    lookback_ns: i64,
    time_ms: i64,
    position: i64,
    function: RangeFunction,
    parameter_bits: [u64; 2],
    range_ns: i64,
    geometry: Geometry,
    end_ms: i64,
    input: Input,
}
impl From<&Request> for Key {
    fn from(request: &Request) -> Self {
        Self {
            scope: request.options.into(),
            lookback_ns: request.lookback_ns,
            time_ms: request.options.time_ms,
            position: request.position,
            function: request.function,
            parameter_bits: request.parameters.map(f64::to_bits),
            range_ns: request.range_ns,
            geometry: request.window.into(),
            end_ms: request.window.end_ms,
            input: request.input,
        }
    }
}

pub(crate) struct Folded<P = Progress> {
    float_samples: Option<crate::statistics::OwnedSamples>,
    pub(crate) value: Option<OwnedValue>,
    pub(crate) annotations: [Option<AnnotationKind>; 6],
    pub(crate) invalid_quantile: bool,
    pub(crate) nonempty: bool,
    progress: Option<P>,
}
impl From<RateOutput> for Folded {
    fn from(output: RateOutput) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value,
            annotations: output.annotations,
            invalid_quantile: false,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl From<FoldResult> for Folded {
    fn from(output: FoldResult) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value,
            annotations: [
                output.annotations[0],
                output.annotations[1],
                None,
                None,
                None,
                None,
            ],
            invalid_quantile: false,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl From<NumericRangeOutput> for Folded {
    fn from(output: NumericRangeOutput) -> Self {
        Self {
            float_samples: output.float_samples,
            value: output.value.map(OwnedValue::Float),
            annotations: [output.annotation, None, None, None, None, None],
            invalid_quantile: output.invalid_quantile,
            nonempty: output.nonempty,
            progress: output.progress,
        }
    }
}
impl Folded {
    pub(crate) fn into_parts(self) -> Option<crate::statistics::ValueParts> {
        self.value.map(|value| crate::statistics::ValueParts {
            value,
            float_samples: self.float_samples,
        })
    }
    pub(crate) fn sample_units(&self) -> u64 {
        self.progress.map_or(0, |progress| progress.sample_units)
    }
}
impl<P> Folded<P> {
    fn map_progress<Q>(self, map: impl FnOnce(P) -> Q) -> Folded<Q> {
        let progress = self.progress.map(map);
        Folded {
            float_samples: self.float_samples,
            value: self.value,
            annotations: self.annotations,
            invalid_quantile: self.invalid_quantile,
            nonempty: self.nonempty,
            progress,
        }
    }
}
impl<P: Copy> Folded<P> {
    fn copy_checked(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        control.check()?;
        let value = self
            .value
            .as_ref()
            .map(|value| {
                OwnedValue::from_sample_checked(
                    value.as_ref(),
                    control,
                    account,
                    MemoryKind::Workspace,
                )
            })
            .transpose()?;
        Ok(Self {
            float_samples: value
                .as_ref()
                .and_then(|value| crate::statistics::hold_float(account, value.as_ref())),
            value,
            annotations: self.annotations,
            invalid_quantile: self.invalid_quantile,
            nonempty: self.nonempty,
            progress: self.progress,
        })
    }
}

struct Snapshot {
    key: Key,
    folded: Folded<Progress<()>>,
}
impl Snapshot {
    fn progress(&self) -> Option<Progress> {
        self.folded
            .progress
            .map(|progress| progress.with_geometry(self.key.geometry))
    }

    fn copy_checked(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Folded, EvaluationError> {
        // Replay must use the geometry of this snapshot, including for an older clock.
        Ok(self
            .folded
            .copy_checked(control, account)?
            .map_progress(|progress| progress.with_geometry(self.key.geometry)))
    }
}
struct Record {
    identity: (usize, usize, usize),
    snapshot: Option<Strong<Snapshot>>,
}
struct AggregateValue {
    key: ReplayKey,
    value: f64,
    drop_name: bool,
    _samples: crate::statistics::OwnedSamples,
}
struct AggregateRecord {
    node: NodeId,
    value: Option<AggregateValue>,
}
struct FoldState {
    records: AccountedVec<Record>,
    aggregates: Option<AccountedBox<AccountedVec<AggregateRecord>>>,
}
pub(crate) struct Folds {
    state: RefCell<FoldState>,
}
impl Folds {
    #[cfg(test)]
    pub(crate) fn completed_aggregates(&self) -> usize {
        self.state
            .borrow()
            .aggregates
            .as_ref()
            .map_or(0, |records| {
                records
                    .iter()
                    .filter(|record| record.value.is_some())
                    .count()
            })
    }

    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            state: RefCell::new(FoldState {
                records: AccountedVec::new(account, MemoryKind::Workspace),
                aggregates: None,
            }),
        }
    }

    fn clear_aggregates(
        state: &mut FoldState,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(records) = &mut state.aggregates {
            while records.pop().is_some() {
                control.check()?;
                account.work(1);
            }
        }
        state.aggregates = None;
        Ok(())
    }

    pub(crate) fn clear(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut state = self.state.borrow_mut();
        Self::clear_aggregates(&mut state, control, account)?;
        let records = &mut state.records;
        while records.pop().is_some() {
            control.check()?;
            account.work(1);
        }
        Ok(())
    }

    pub(crate) fn retain_numeric(
        &self,
        owner: NodeId,
        selector: NodeId,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let mut state = self.state.borrow_mut();
        Self::clear_aggregates(&mut state, control, account)?;
        let records = &mut state.records;
        // Preserve sorted identities without another allocation or repeated shifts.
        let mut kept = 0;
        for position in 0..records.len() {
            control.check()?;
            account.work(1);
            let record = &records[position];
            let retain = record.identity.0 == owner.0
                && record.identity.1 == selector.0
                && record.snapshot.as_ref().is_some_and(|snapshot| {
                    matches!(snapshot.key.function, RangeFunction::Numeric(_))
                        && matches!(snapshot.folded.value, None | Some(OwnedValue::Float(_)))
                });
            if retain {
                if kept != position {
                    records.swap(kept, position);
                }
                kept += 1;
            } else {
                records[position].snapshot = None;
            }
        }
        while records.len() > kept {
            control.check()?;
            account.work(1);
            drop(records.pop());
        }
        Ok(())
    }

    fn position<T>(
        records: &[T],
        compare: impl Fn(&T) -> std::cmp::Ordering,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Result<usize, usize>, Interrupted> {
        let (mut first, mut end) = (0, records.len());
        while first < end {
            control.check()?;
            account.work(1);
            let middle = first + (end - first) / 2;
            match compare(&records[middle]) {
                std::cmp::Ordering::Less => first = middle + 1,
                std::cmp::Ordering::Greater => end = middle,
                std::cmp::Ordering::Equal => return Ok(Ok(middle)),
            }
        }
        Ok(Err(first))
    }

    pub(crate) fn aggregate_value(
        &self,
        node: NodeId,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<(f64, bool)>, Interrupted> {
        control.check()?;
        let mut state = self.state.borrow_mut();
        let Some(records) = &mut state.aggregates else {
            return Ok(None);
        };
        let Ok(position) = Self::position(
            records,
            |record| record.node.0.cmp(&node.0),
            control,
            account,
        )?
        else {
            return Ok(None);
        };
        let record = &mut records[position];
        if record.value.as_ref().is_some_and(|value| value.key != key) {
            record.value = None;
        }
        control.check()?;
        Ok(record
            .value
            .as_ref()
            .map(|value| (value.value, value.drop_name)))
    }

    pub(crate) fn remember_aggregate(
        &self,
        node: NodeId,
        key: ReplayKey,
        value: f64,
        drop_name: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        let mut state = self.state.borrow_mut();
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let records = state.aggregates.get_or_insert_with(|| {
            AccountedBox::new(AccountedVec::with_capacity_in(1, Some(&context)), &context)
        });
        let position = Self::position(
            records,
            |record| record.node.0.cmp(&node.0),
            control,
            account,
        )?;
        let position = match position {
            Ok(position) => {
                control.check()?;
                position
            }
            Err(position) => {
                account.work(records.len().saturating_sub(position) as u64);
                control.check()?;
                records.insert(position, AggregateRecord { node, value: None });
                position
            }
        };
        let value = AggregateValue {
            key,
            value,
            drop_name,
            _samples: crate::statistics::OwnedSamples::new(account, 1),
        };
        account.work(1);
        control.check()?;
        records[position].value = Some(value);
        Ok(())
    }

    pub(crate) fn evaluate(
        &self,
        request: Request,
        control: &Control,
        account: &QueryAccounting,
        evaluate: impl FnOnce(MatrixWindow) -> Result<Folded, EvaluationError>,
    ) -> Result<Folded, EvaluationError> {
        control.check()?;
        let key = Key::from(&request);
        let identity = (request.owner.0, request.selector.0, request.ordinal);
        let (cached, progress) = {
            let mut state = self.state.borrow_mut();
            let records = &mut state.records;
            match Self::position(
                records,
                |record| record.identity.cmp(&identity),
                control,
                account,
            )? {
                Ok(index) => match &records[index].snapshot {
                    Some(snapshot) if snapshot.key == key => (Some(snapshot.clone()), None),
                    _ => {
                        let prior = records[index].snapshot.take();
                        let progress = prior
                            .as_ref()
                            .filter(|snapshot| snapshot.key.scope == key.scope)
                            .and_then(|snapshot| snapshot.progress());
                        (None, progress)
                    }
                },
                Err(_) => (None, None),
            }
        };
        if let Some(cached) = cached {
            return cached.copy_checked(control, account);
        }
        let mut folded = evaluate(request.window.with_progress(progress))?;
        if folded.float_samples.is_none() {
            folded.float_samples = folded
                .value
                .as_ref()
                .and_then(|value| crate::statistics::hold_float(account, value.as_ref()));
        }
        let folded = folded.map_progress(|progress| progress.without_geometry(key.geometry));
        let snapshot = Strong::new_in(
            Snapshot { key, folded },
            Some(&AllocationContext::new(account, MemoryKind::Workspace)),
        );
        {
            let mut state = self.state.borrow_mut();
            let records = &mut state.records;
            match Self::position(
                records,
                |record| record.identity.cmp(&identity),
                control,
                account,
            )? {
                Ok(index) => records[index].snapshot = Some(snapshot.clone()),
                Err(index) => {
                    account.work(records.len().saturating_sub(index) as u64 + 1);
                    records.insert(
                        index,
                        Record {
                            identity,
                            snapshot: Some(snapshot.clone()),
                        },
                    );
                }
            }
        }
        snapshot.copy_checked(control, account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{CUSTOM_BUCKETS_SCHEMA, FloatHistogram, Span},
        range_numeric::NumericRangeFunction,
        test_storage::interrupt_during,
        value::SampleValueRef,
    };
    use std::cell::Cell;

    fn request(time_ms: i64, parameter: f64) -> Request {
        let window = MatrixWindow {
            first_end_ms: 1000,
            end_ms: time_ms,
            range_ms: 1000,
            buffer_ms: None,
            step_ms: 2000,
            data_bounds: TimeBounds {
                start_ms: 0,
                end_ms: 100_000_000,
            },
            statistics: false,
            reduce_delta: true,
            resume: None,
        };
        Request {
            owner: NodeId(2),
            lookback_ns: 1_000_000_000,
            selector: NodeId(0),
            ordinal: 0,
            options: EvaluationOptions {
                time_ms,
                start_ms: 1000,
                end_ms: 100_000_000,
                step_ms: 2000,
                lookback_ms: 1000,
                start_timestamps: false,
                delayed_name_removal: true,
            },
            position: (time_ms - 1000) / 2000,
            function: RangeFunction::Numeric(NumericRangeFunction::Quantile),
            parameters: [parameter, 0.0],
            range_ns: 1_000_000_000,
            input: NumericRangeInput {
                bounds: Some(TimeBounds {
                    start_ms: time_ms - 999,
                    end_ms: time_ms,
                }),
                seek_ms: time_ms - 1000,
                matrix: Some(window),
            }
            .into(),
            window,
        }
    }
    fn completed(value: Option<OwnedValue>) -> Folded {
        Folded {
            float_samples: None,
            value,
            annotations: [
                Some(AnnotationKind::HistogramIgnoredInMixedRange),
                None,
                None,
                None,
                None,
                None,
            ],
            invalid_quantile: true,
            nonempty: true,
            progress: None,
        }
    }

    fn aggregate_key(time_ms: i64) -> ReplayKey {
        let request = request(time_ms, 0.0);
        ReplayKey::new(
            request.options,
            request.position,
            request.lookback_ns,
            request.window.data_bounds,
        )
    }

    #[test]
    fn completed_single_group_values_replace_keys_and_keep_independent_leases() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        assert!(folds.state.borrow().aggregates.is_none());
        let bits = [
            0,
            1_u64 << 63,
            0x7ff0_0000_0000_0000,
            0xfff0_0000_0000_0000,
            0x7ff8_0000_0000_0042,
        ];
        let mut capacity = None;
        for clock in 0..1024 {
            let key = aggregate_key(1000 + clock * 2000);
            for node in [NodeId(9), NodeId(2)] {
                assert!(
                    folds
                        .aggregate_value(node, key, &control, &account)
                        .unwrap()
                        .is_none()
                );
                let value = f64::from_bits(bits[(clock as usize + node.0) % bits.len()]);
                let drop_name = node.0 == 9;
                folds
                    .remember_aggregate(node, key, value, drop_name, &control, &account)
                    .unwrap();
                let actual = folds
                    .aggregate_value(node, key, &control, &account)
                    .unwrap()
                    .unwrap();
                assert_eq!((actual.0.to_bits(), actual.1), (value.to_bits(), drop_name));
            }
            let state = folds.state.borrow();
            let records = state.aggregates.as_ref().unwrap();
            assert_eq!(records.len(), 2);
            assert_eq!(account.current_samples(), 2);
            if let Some(capacity) = capacity {
                assert_eq!(records.capacity(), capacity);
            }
            capacity = Some(records.capacity());
        }
        let key = aggregate_key(1000 + 1023 * 2000);
        let (value, _) = folds
            .aggregate_value(NodeId(2), key, &control, &account)
            .unwrap()
            .unwrap();
        let deliveries = [
            crate::statistics::ValueParts::new(OwnedValue::Float(value), &account),
            crate::statistics::ValueParts::new(OwnedValue::Float(value), &account),
        ];
        assert_eq!(account.current_samples(), 4);
        folds.clear(&control, &account).unwrap();
        assert!(folds.state.borrow().aggregates.is_none());
        drop(folds);
        assert_eq!(account.current_samples(), 2);
        for delivery in &deliveries {
            let OwnedValue::Float(actual) = &delivery.value else {
                unreachable!()
            };
            assert_eq!(actual.to_bits(), value.to_bits());
        }
        drop(deliveries);
        assert_eq!(account.current_samples(), 0);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn completed_single_group_values_keep_control_and_phase_boundaries() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let node = NodeId(9);
        let key = aggregate_key(1000);
        let cancelled = Control::default();
        cancelled.cancellation.cancel();
        assert!(matches!(
            folds.remember_aggregate(node, key, 1.0, false, &cancelled, &account),
            Err(Interrupted::Cancelled)
        ));
        assert!(folds.state.borrow().aggregates.is_none());
        let publish = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &publish, 1, || {
            folds.remember_aggregate(node, key, 7.0, false, &publish, &account)
        });
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        assert_eq!(folds.completed_aggregates(), 0);
        assert_eq!(account.current_samples(), 0);
        assert!(
            folds
                .aggregate_value(node, key, &control, &account)
                .unwrap()
                .is_none()
        );
        folds
            .remember_aggregate(node, key, -0.0, true, &control, &account)
            .unwrap();
        let replace = Control::default();
        let result = crate::test_storage::interrupt_after_work(&account, &replace, 1, || {
            folds.remember_aggregate(node, key, 99.0, false, &replace, &account)
        });
        assert!(matches!(result, Err(Interrupted::Cancelled)));
        let value = folds
            .aggregate_value(node, key, &control, &account)
            .unwrap()
            .unwrap();
        assert_eq!((value.0.to_bits(), value.1), ((-0.0_f64).to_bits(), true));
        let hit = Control::default();
        assert!(matches!(
            crate::test_storage::interrupt_after_work(&account, &hit, 1, || {
                folds.aggregate_value(node, key, &hit, &account)
            }),
            Err(Interrupted::Cancelled)
        ));
        assert_eq!(account.current_samples(), 1);
        assert!(
            folds
                .aggregate_value(node, aggregate_key(3000), &control, &account)
                .unwrap()
                .is_none()
        );
        assert_eq!(account.current_samples(), 0);
        folds
            .remember_aggregate(node, key, 1.0, false, &control, &account)
            .unwrap();
        drop(
            folds
                .evaluate(request(1000, 0.0), &control, &account, |_| {
                    Ok(completed(Some(OwnedValue::Float(2.0))))
                })
                .unwrap(),
        );
        folds
            .retain_numeric(NodeId(2), NodeId(0), &control, &account)
            .unwrap();
        assert!(folds.state.borrow().aggregates.is_none());
        assert_eq!(folds.state.borrow().records.len(), 1);
        assert_eq!(account.current_samples(), 1);
        println!(
            "single-group-layout legacy_folds={} folds={} state={} raw_record={} aggregate_record={} aggregate_owner={}",
            size_of::<RefCell<AccountedVec<Record>>>(),
            size_of::<Folds>(),
            size_of::<FoldState>(),
            size_of::<Record>(),
            size_of::<AggregateRecord>(),
            AccountedBox::<AccountedVec<AggregateRecord>>::allocation_bytes()
        );
        folds.clear(&control, &account).unwrap();
        drop(folds);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn cached_progress_shares_geometry_with_its_key() {
        struct CompleteSnapshot {
            _key: Key,
            _folded: Folded,
        }
        println!(
            "fold-geometry-layout geometry={} progress={} folded={} key={} snapshot={} complete_snapshot={} shared={} complete_shared={} folds={} state={} record={}",
            size_of::<Geometry>(),
            size_of::<Progress>(),
            size_of::<Folded>(),
            size_of::<Key>(),
            size_of::<Snapshot>(),
            size_of::<CompleteSnapshot>(),
            Strong::<Snapshot>::allocation_bytes(),
            Strong::<CompleteSnapshot>::allocation_bytes(),
            size_of::<Folds>(),
            size_of::<FoldState>(),
            size_of::<Record>()
        );
        assert_eq!(
            size_of::<Snapshot>() + size_of::<Geometry>(),
            size_of::<CompleteSnapshot>()
        );
        assert_eq!(
            Strong::<Snapshot>::allocation_bytes() + size_of::<Geometry>() as u64,
            Strong::<CompleteSnapshot>::allocation_bytes()
        );
    }

    fn progress_sample(index: usize) -> crate::value::Sample {
        crate::value::Sample {
            time_ms: [1200, 1800, 2100, 3200, 3800, 4100, 5200, 5800][index],
            start_time_ms: 0,
            value: crate::value::SampleValue::Float(index as f64),
        }
    }

    fn progress_request(time_ms: i64) -> Request {
        let mut result = request(time_ms, 0.0);
        result.window.first_end_ms = 2000;
        result
    }

    fn scan_progress(window: MatrixWindow, account: &QueryAccounting) -> Progress {
        scan_progress_kind(window, account, false)
    }

    fn progress_native_sample(index: usize) -> crate::value::Sample {
        let mut sample = progress_sample(index);
        sample.value = crate::value::SampleValue::FloatHistogram(FloatHistogram {
            count: 1.0,
            sum: index as f64,
            ..FloatHistogram::default()
        });
        sample
    }

    fn scan_progress_kind(
        window: MatrixWindow,
        account: &QueryAccounting,
        native: bool,
    ) -> Progress {
        let source = crate::test_storage::GeneratedSeries::new(
            8,
            1,
            if native {
                progress_native_sample
            } else {
                progress_sample
            },
        );
        crate::matrix_reader::scan::<_, crate::storage::StorageError>(
            &source,
            TimeBounds {
                start_ms: window.end_ms - window.range_ms + 1,
                end_ms: window.end_ms,
            },
            Some(window),
            &Control::default(),
            account,
            |_| Ok(()),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn cached_progress_preserves_fields_and_prior_geometry() {
        for native in [false, true] {
            for absent in [false, true] {
                for changed in [false, true] {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let folds = Folds::new(&account);
                    let physical = progress_request(4000).window;
                    let expected = scan_progress_kind(physical, &account, native);
                    let next = progress_request(6000).window;
                    assert!(expected.resume(next).is_some());
                    for warm in [false, true] {
                        let output = folds
                            .evaluate(progress_request(4000), &control, &account, |window| {
                                assert!(
                                    !warm,
                                    "completed fold must preserve progress without rereading"
                                );
                                let mut result =
                                    completed((!absent).then_some(OwnedValue::Float(3.0)));
                                result.progress =
                                    Some(scan_progress_kind(window, &account, native));
                                Ok(result)
                            })
                            .unwrap();
                        assert_eq!(output.progress, Some(expected));
                        assert_eq!(output.sample_units(), expected.sample_units);
                        assert_eq!(output.value.is_none(), absent);
                        assert!(output.nonempty && output.invalid_quantile);
                        assert_eq!(
                            output.annotations[0],
                            Some(AnnotationKind::HistogramIgnoredInMixedRange)
                        );
                    }
                    let mut next_request = progress_request(6000);
                    next_request.window.statistics = changed;
                    let next_output = folds
                        .evaluate(next_request, &control, &account, |window| {
                            assert_eq!(
                                window.resume,
                                if changed { None } else { expected.resume(next) }
                            );
                            let mut result = completed(None);
                            result.progress = Some(scan_progress_kind(window, &account, native));
                            Ok(result)
                        })
                        .unwrap();
                    let mut warm_request = progress_request(6000);
                    warm_request.window.statistics = changed;
                    let warm = folds
                        .evaluate(warm_request, &control, &account, |_| {
                            panic!("adjacent fold must retain its progress")
                        })
                        .unwrap();
                    let expected_next = scan_progress_kind(
                        MatrixWindow {
                            statistics: changed,
                            ..next
                        }
                        .with_progress(Some(expected)),
                        &account,
                        native,
                    );
                    assert_eq!(next_output.progress, Some(expected_next));
                    assert_eq!(warm.progress, Some(expected_next));
                    drop(warm);
                    drop(next_output);
                    drop(folds);
                    account.snapshot().assert_memory_released();
                    drop(account);
                    engine.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn mismatched_private_progress_releases_native_and_sample_owners() {
        for native in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let folds = Folds::new(&account);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                folds.evaluate(progress_request(4000), &control, &account, |mut window| {
                    window.statistics = true;
                    let histogram = FloatHistogram {
                        schema: CUSTOM_BUCKETS_SCHEMA,
                        custom_values: Some(vec![1.0, 2.0].into()),
                        ..FloatHistogram::default()
                    };
                    let value = OwnedValue::from_sample_checked(
                        if native {
                            SampleValueRef::FloatHistogram(histogram.as_ref())
                        } else {
                            SampleValueRef::Float(2.0)
                        },
                        &control,
                        &account,
                        MemoryKind::Workspace,
                    )?;
                    let mut result = completed(Some(value));
                    result.progress = Some(scan_progress(window, &account));
                    Ok(result)
                })
            }));
            let panicked = outcome.is_err();
            drop(outcome);
            drop(folds);
            assert_eq!(account.current_samples(), 0);
            account.snapshot().assert_memory_released();
            drop(account);
            engine.snapshot().assert_memory_released();
            assert!(
                panicked,
                "unrelated progress must not be stored under a different geometry"
            );
        }
    }

    #[test]
    fn exact_parameter_bits_clock_and_lifecycle_keep_callbacks_authoritative() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let calls = Cell::new(0);
        for (time, parameter, expected_calls) in [
            (1000, 0.0, 1),
            (1000, 0.0, 1),
            (1000, -0.0, 2),
            (3000, -0.0, 3),
            (3000, -0.0, 3),
            (1000, -0.0, 4),
        ] {
            let output = folds
                .evaluate(request(time, parameter), &control, &account, |_| {
                    calls.set(calls.get() + 1);
                    Ok(completed(None))
                })
                .unwrap();
            assert!(output.value.is_none());
            assert!(output.nonempty && output.invalid_quantile);
            assert_eq!(
                output.annotations[0],
                Some(AnnotationKind::HistogramIgnoredInMixedRange)
            );
            assert_eq!(calls.get(), expected_calls);
        }
        folds.clear(&control, &account).unwrap();
        folds
            .evaluate(request(1000, -0.0), &control, &account, |_| {
                calls.set(calls.get() + 1);
                Ok(completed(None))
            })
            .unwrap();
        assert_eq!(calls.get(), 5);
        folds.clear(&control, &account).unwrap();
        for _ in 0..2 {
            let failure = folds.evaluate(request(1000, -0.0), &control, &account, |_| {
                calls.set(calls.get() + 1);
                Err(EvaluationError::Invalid("source acquisition failure"))
            });
            assert!(matches!(
                failure,
                Err(EvaluationError::Invalid("source acquisition failure"))
            ));
        }
        assert_eq!(calls.get(), 7);
        drop(folds);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn cached_histogram_delivery_preserves_fields_and_independent_ownership() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let histogram = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            zero_threshold: 0.25,
            zero_count: -0.0,
            sum: f64::from_bits(0xfff8_0000_0000_0045),
            negative_spans: Some(Vec::new().into()),
            negative_buckets: Some(Vec::new().into()),
            custom_values: Some(vec![1.0].into()),
            ..FloatHistogram::default()
        };
        let expected = histogram.as_ref();
        let first = folds.evaluate(request(1000, 0.0), &control, &account, |_| {
            Ok(completed(Some(OwnedValue::from_sample_checked(
                SampleValueRef::FloatHistogram(histogram.as_ref()),
                &control,
                &account,
                MemoryKind::Workspace,
            )?)))
        });
        let first = first.unwrap();
        let second = folds
            .evaluate(request(1000, 0.0), &control, &account, |_| {
                panic!("completed fold must not reacquire storage")
            })
            .unwrap();
        for output in [&first, &second] {
            let Some(OwnedValue::Histogram(histogram)) = &output.value else {
                panic!("native fold")
            };
            let actual = histogram.as_ref();
            assert_eq!(actual.schema, expected.schema);
            assert_eq!(
                actual.zero_threshold.to_bits(),
                expected.zero_threshold.to_bits()
            );
            assert_eq!(actual.zero_count.to_bits(), expected.zero_count.to_bits());
            assert_eq!(actual.sum.to_bits(), expected.sum.to_bits());
            assert_eq!(actual.negative_spans, expected.negative_spans);
            assert_eq!(actual.negative_buckets, expected.negative_buckets);
            assert_eq!(actual.custom_values, expected.custom_values);
        }
        drop(folds);
        assert!(first.value.is_some() && second.value.is_some());
        drop(first);
        drop(second);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn replacing_clocks_retains_one_fold_and_constant_requested_bytes() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let control = Control::default();
        let mut peak = None;
        for index in 0..4096 {
            let time_ms = 1000 + 2000 * index;
            let output = folds
                .evaluate(request(time_ms, 0.0), &control, &account, |_| {
                    Ok(completed(Some(OwnedValue::Float(time_ms as f64))))
                })
                .unwrap();
            drop(output);
            let current = account.snapshot().memory[MemoryKind::Workspace as usize].peak;
            if let Some(peak) = peak {
                assert_eq!(current, peak);
            }
            peak = Some(current);
            assert_eq!(folds.state.borrow().records.len(), 1);
        }
        drop(folds);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn interrupted_warm_histogram_copy_releases_scratch_and_keeps_the_snapshot() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let folds = Folds::new(&account);
        let histogram = FloatHistogram {
            schema: CUSTOM_BUCKETS_SCHEMA,
            count: 100_000.0,
            sum: 100_000.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 100_000,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0; 100_000].into()),
            custom_values: Some((1..100_000).map(f64::from).collect::<Vec<_>>().into()),
            ..FloatHistogram::default()
        };
        let control = Control::default();
        let first = folds
            .evaluate(request(1000, 0.0), &control, &account, |_| {
                Ok(completed(Some(OwnedValue::from_sample_checked(
                    SampleValueRef::FloatHistogram(histogram.as_ref()),
                    &control,
                    &account,
                    MemoryKind::Workspace,
                )?)))
            })
            .unwrap();
        drop(first);
        let before = account.snapshot();
        let failure = interrupt_during(&account, &control, || {
            folds.evaluate(request(1000, 0.0), &control, &account, |_| {
                panic!("warm fold must not reacquire storage")
            })
        });
        assert!(matches!(
            failure,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        let after = account.snapshot();
        for (before, after) in before.memory.iter().zip(&after.memory) {
            assert_eq!(after.current, before.current);
            assert_eq!(
                after.acquired - before.acquired,
                after.released - before.released
            );
        }
        assert!(after.work - before.work >= 8192);
        let workspace = MemoryKind::Workspace as usize;
        assert!(after.memory[workspace].acquired - before.memory[workspace].acquired >= 800_000);

        let recovered = folds
            .evaluate(request(1000, 0.0), &Control::default(), &account, |_| {
                panic!("cancelled delivery must keep the completed snapshot")
            })
            .unwrap();
        let Some(OwnedValue::Histogram(value)) = &recovered.value else {
            panic!("native cached fold")
        };
        assert_eq!(
            value.as_ref().positive_buckets,
            histogram.as_ref().positive_buckets
        );
        assert_eq!(
            value.as_ref().custom_values,
            histogram.as_ref().custom_values
        );
        assert_eq!(folds.state.borrow().records.len(), 1);
        drop(folds);
        assert!(recovered.value.is_some());
        drop(recovered);
        account.snapshot().assert_memory_released();
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
