// SPDX-License-Identifier: GPL-3.0-or-later
// Bounded pair replay follows pinned promql/engine.go:3172-3462; see NOTICE.
use crate::{
    accounting::{AccountedBox, AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    binary_labels,
    binary_value::{self, Diagnostic},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Cardinality, NodeId, VectorMatching},
        token::TokenKind,
    },
    label_hash,
    position_index::{PositionIndex, SignatureIndex},
    result::{OwnedLabels, OwnedValue, append_bytes_checked},
    sample_labels::SampleLabels,
    storage::{Control, Interrupted},
};
use std::cell::{Cell, RefCell};

pub(crate) struct Side {
    pub(crate) rows: AccountedVec<Descriptor>,
    signatures: AccountedVec<usize>,
    ordinal_positions: Option<AccountedBox<AccountedVec<usize>>>,
}
struct Key {
    bytes: AccountedVec<u8>,
    hash: u64,
}
struct CacheEntry {
    key: Key,
    labels: OwnedLabels,
}
pub(crate) struct Cache {
    entries: AccountedVec<CacheEntry>,
    index: PositionIndex,
}
impl Cache {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            entries: AccountedVec::new(account, MemoryKind::Workspace),
            index: PositionIndex::new(account),
        }
    }
    fn position(
        &self,
        key: &[u8],
        hash: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        self.index.find(
            hash,
            |position| {
                Ok(self.entries[position].key.hash == hash
                    && binary_labels::equal(
                        &self.entries[position].key.bytes,
                        key,
                        control,
                        account,
                    )?)
            },
            control,
            account,
        )
    }
}
pub(crate) struct Plan {
    pub(crate) sides: [Side; 2],
    pub(crate) cache: Cache,
    signature_count: usize,
    replay: Option<AccountedBox<Replay>>,
}

struct Replay {
    rows: ReplayRows,
    completed: RefCell<Option<AccountedBox<CompletedStep>>>,
}
enum ReplayRows {
    Pairs(AccountedVec<[usize; 2]>),
    PairCandidate,
    PairDiscovery(AccountedBox<crate::pair_discovery::Plan>),
    // Certified set subtrees serialize Steps, so a scope needs only one successful prefix.
    SetDiscovery { prefix: Cell<Option<usize>> },
}
struct CompletedStep {
    key: ReplayKey,
    members: AccountedVec<u64>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayKey {
    clock: [i64; 9],
    start_timestamps: bool,
    delayed_name_removal: bool,
}
impl ReplayKey {
    pub(crate) fn new(
        options: EvaluationOptions,
        step_index: i64,
        lookback_ns: i64,
        data_bounds: crate::storage::TimeBounds,
    ) -> Self {
        Self {
            clock: [
                options.time_ms,
                options.start_ms,
                options.end_ms,
                options.step_ms,
                options.lookback_ms,
                step_index,
                lookback_ns,
                data_bounds.start_ms,
                data_bounds.end_ms,
            ],
            start_timestamps: options.start_timestamps,
            delayed_name_removal: options.delayed_name_removal,
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Progress<'a> {
    pub(crate) key: &'a ReplayKey,
    pub(crate) target: Option<usize>,
}
#[derive(Clone, Copy)]
pub(crate) struct Preparation<'a> {
    pub(crate) node: NodeId,
    pub(crate) cache: &'a RefCell<Cache>,
}
impl Side {
    fn new(
        rows: AccountedVec<Descriptor>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, Interrupted> {
        let mut ordered = true;
        for pair in rows.windows(2) {
            control.check()?;
            account.work(1);
            ordered &= pair[0].ordinal <= pair[1].ordinal;
        }
        let ordinal_positions = if ordered {
            None
        } else {
            let mut positions = AccountedVec::new(account, MemoryKind::Workspace);
            for position in 0..rows.len() {
                control.check()?;
                account.work(1);
                positions.push(position);
            }
            crate::evaluator_order::sort(&mut positions, control, account, |left, right| {
                (rows[*left].ordinal, *left).cmp(&(rows[*right].ordinal, *right))
            })?;
            Some(AccountedBox::new(
                positions,
                &AllocationContext::new(account, MemoryKind::Workspace),
            ))
        };
        Ok(Self {
            rows,
            signatures: AccountedVec::new(account, MemoryKind::Workspace),
            ordinal_positions,
        })
    }
    fn row_position(&self, position: usize) -> usize {
        self.ordinal_positions
            .as_ref()
            .map_or(position, |rows| rows[position])
    }
}
impl Plan {
    pub(crate) fn new(
        left: AccountedVec<Descriptor>,
        right: AccountedVec<Descriptor>,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let mut keys = AccountedVec::<Key>::new(account, MemoryKind::Workspace);
        let mut index = PositionIndex::new(account);
        let mut sides = [
            Side::new(left, control, account)?,
            Side::new(right, control, account)?,
        ];
        for side in &mut sides {
            for row in &*side.rows {
                let mut key = AccountedVec::new(account, MemoryKind::Workspace);
                binary_labels::append(&mut key, &row.labels, Some(matching), control, account)?;
                let hash = label_hash::bytes(&key, control, account)?;
                let found = index.find(
                    hash,
                    |position| {
                        Ok(keys[position].hash == hash
                            && binary_labels::equal(&key, &keys[position].bytes, control, account)?)
                    },
                    control,
                    account,
                )?;
                let signature = if let Some(position) = found {
                    position
                } else {
                    let position = keys.len();
                    keys.push(Key { bytes: key, hash });
                    index.insert_new(
                        hash,
                        position,
                        |prior| Ok(keys[prior].hash),
                        control,
                        account,
                    )?;
                    position
                };
                side.signatures.push(signature);
            }
        }
        Ok(Self {
            sides,
            cache: Cache::new(account),
            signature_count: keys.len(),
            replay: None,
        })
    }
    pub(crate) fn certify_replay(
        &mut self,
        required: &[usize],
        cardinality: Cardinality,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if cardinality == Cardinality::ManyToMany {
            return Ok(());
        }
        let many = usize::from(cardinality == Cardinality::OneToMany);
        for side in &self.sides {
            if !crate::scheduled_expression::unique_rows(&side.rows, control, account)? {
                return Ok(());
            }
            for row in &*side.rows {
                control.check()?;
                account.work(1);
                if row.kinds != 1 {
                    return Ok(());
                }
            }
        }
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let missing = [usize::MAX; 2];
        let mut positions = AccountedVec::with_capacity_in(self.signature_count, Some(&context));
        for _ in 0..self.signature_count {
            control.check()?;
            account.work(1);
            positions.push(missing);
        }
        for side in 0..2 {
            for (row, signature) in self.sides[side].signatures.iter().enumerate() {
                control.check()?;
                account.work(1);
                if positions[*signature][side] != usize::MAX
                    && (cardinality == Cardinality::OneToOne || side != many)
                {
                    return Ok(());
                }
                positions[*signature][side] = row;
            }
        }
        let mut pairs = AccountedVec::with_capacity_in(self.cache.entries.len(), Some(&context));
        for _ in &*self.cache.entries {
            control.check()?;
            account.work(1);
            pairs.push(missing);
        }
        {
            let mut bind = |pair: [usize; 2]| -> Result<bool, EvaluationError> {
                if pair.contains(&usize::MAX) {
                    return Ok(true);
                }
                let mut key = AccountedVec::new(account, MemoryKind::Workspace);
                // Frozen keys follow evaluation's many/one order; positions retain original sides.
                for side in [many, 1 - many] {
                    binary_labels::append(
                        &mut key,
                        &self.sides[side].rows[pair[side]].labels,
                        None,
                        control,
                        account,
                    )?;
                }
                let hash = label_hash::bytes(&key, control, account)?;
                if let Some(ordinal) = self.cache.position(&key, hash, control, account)? {
                    if pairs[ordinal] != missing {
                        return Ok(false);
                    }
                    pairs[ordinal] = pair;
                }
                Ok(true)
            };
            if cardinality == Cardinality::OneToOne {
                for pair in &*positions {
                    control.check()?;
                    account.work(1);
                    if !bind(*pair)? {
                        return Ok(());
                    }
                }
            } else {
                for (row, signature) in self.sides[many].signatures.iter().enumerate() {
                    control.check()?;
                    account.work(1);
                    let mut pair = positions[*signature];
                    pair[many] = row;
                    if !bind(pair)? {
                        return Ok(());
                    }
                }
            }
        }
        for ordinal in required {
            control.check()?;
            account.work(1);
            if pairs.get(*ordinal).is_none_or(|pair| *pair == missing) {
                return Ok(());
            }
        }
        drop(positions);
        control.check()?;
        if let Some(replay) = self.replay.as_deref_mut() {
            replay.rows = ReplayRows::Pairs(pairs);
            replay.completed.get_mut().take();
        } else {
            self.replay = Some(AccountedBox::new(
                Replay {
                    rows: ReplayRows::Pairs(pairs),
                    completed: RefCell::new(None),
                },
                &context,
            ));
        }
        Ok(())
    }
    pub(crate) fn prepare_pair_discovery(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        control.check()?;
        self.replay = Some(AccountedBox::new(
            Replay {
                rows: ReplayRows::PairCandidate,
                completed: RefCell::new(None),
            },
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn has_pair_candidate(&self) -> bool {
        self.replay
            .as_ref()
            .is_some_and(|replay| matches!(replay.rows, ReplayRows::PairCandidate))
    }
    pub(crate) fn discard_pair_candidate(&mut self) {
        if self.has_pair_candidate() {
            self.replay = None;
        }
    }
    #[cfg(test)]
    pub(crate) fn has_completed_pair_discovery(&self) -> bool {
        self.has_pair_discovery()
            && self
                .replay
                .as_ref()
                .is_some_and(|replay| replay.completed.borrow().is_some())
    }
    pub(crate) fn certify_set_discovery(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.sides.iter().map(|side| side.rows.len()).sum::<usize>() < 2 {
            return Ok(());
        }
        for side in &self.sides {
            if !crate::scheduled_expression::unique_rows(&side.rows, control, account)? {
                return Ok(());
            }
            let mut prior = None;
            for position in 0..side.rows.len() {
                control.check()?;
                account.work(1);
                let row = &side.rows[side.row_position(position)];
                if row.kinds != 1 || prior == Some(row.ordinal) {
                    return Ok(());
                }
                prior = Some(row.ordinal);
            }
        }
        control.check()?;
        self.replay = Some(AccountedBox::new(
            Replay {
                rows: ReplayRows::SetDiscovery {
                    prefix: Cell::new(None),
                },
                completed: RefCell::new(None),
            },
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        Ok(())
    }
    pub(crate) fn certify_pair_discovery(
        &mut self,
        cardinality: Cardinality,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let Some(mut replay) = self.replay.take() else {
            return Ok(());
        };
        let rows = std::mem::replace(
            &mut replay.rows,
            ReplayRows::Pairs(AccountedVec::new(account, MemoryKind::Workspace)),
        );
        let ReplayRows::Pairs(pairs) = rows else {
            if !matches!(rows, ReplayRows::PairCandidate) {
                replay.rows = rows;
                self.replay = Some(replay);
            }
            return Ok(());
        };
        let mut hashes = AccountedVec::new(account, MemoryKind::Workspace);
        for entry in &*self.cache.entries {
            control.check()?;
            account.work(1);
            hashes.push(label_hash::full(&entry.labels, control, account)?);
        }
        let many = usize::from(cardinality == Cardinality::OneToMany);
        let Some(mut plan) = crate::pair_discovery::Plan::new(
            pairs,
            hashes,
            many,
            self.sides[many].rows.len(),
            control,
            account,
        )?
        else {
            return Ok(());
        };
        plan.certify_frontier(
            [&self.sides[0].signatures, &self.sides[1].signatures],
            self.signature_count,
            many,
            control,
            account,
        )?;
        control.check()?;
        replay.rows = ReplayRows::PairDiscovery(AccountedBox::new(
            plan,
            &AllocationContext::new(account, MemoryKind::Workspace),
        ));
        replay.completed.borrow_mut().take();
        self.replay = Some(replay);
        Ok(())
    }
    pub(crate) fn has_set_discovery(&self) -> bool {
        self.replay
            .as_ref()
            .is_some_and(|replay| matches!(replay.rows, ReplayRows::SetDiscovery { .. }))
    }
    pub(crate) fn has_pair_discovery(&self) -> bool {
        self.replay
            .as_ref()
            .is_some_and(|replay| matches!(replay.rows, ReplayRows::PairDiscovery(_)))
    }
    pub(crate) fn clear_discovery(&self) {
        if let Some(replay) = &self.replay {
            match &replay.rows {
                ReplayRows::SetDiscovery { prefix } => prefix.set(None),
                ReplayRows::PairDiscovery(plan) => plan.clear(),
                ReplayRows::Pairs(_) | ReplayRows::PairCandidate => return,
            }
            replay.completed.borrow_mut().take();
        }
    }
    fn pair_discovery(&self, key: &ReplayKey) -> Option<&crate::pair_discovery::Plan> {
        let replay = self.replay.as_ref()?;
        let ReplayRows::PairDiscovery(plan) = &replay.rows else {
            return None;
        };
        replay
            .completed
            .borrow()
            .as_ref()
            .is_some_and(|completed| completed.key == *key)
            .then_some(plan)
    }
    fn set_prefix(&self, key: &ReplayKey) -> Option<usize> {
        let replay = self.replay.as_ref()?;
        let ReplayRows::SetDiscovery { prefix } = &replay.rows else {
            return None;
        };
        replay
            .completed
            .borrow()
            .as_ref()
            .filter(|completed| completed.key == *key)?;
        prefix.get()
    }
    fn extend_set_prefix(
        &self,
        key: &ReplayKey,
        ordinal: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let Some(replay) = &self.replay else {
            return Ok(());
        };
        let ReplayRows::SetDiscovery { prefix } = &replay.rows else {
            return Ok(());
        };
        control.check()?;
        account.work(1);
        if replay
            .completed
            .borrow()
            .as_ref()
            .is_some_and(|completed| completed.key == *key)
        {
            control.check()?;
            prefix.set(Some(
                prefix.get().map_or(ordinal, |prior| prior.max(ordinal)),
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn has_completed_set_discovery(&self) -> bool {
        self.has_set_discovery()
            && self
                .replay
                .as_ref()
                .expect("set discovery owner")
                .completed
                .borrow()
                .is_some()
    }
    fn position(
        &self,
        side: usize,
        sample: &EvaluatedSample<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, EvaluationError> {
        let side = &self.sides[side];
        let (mut left, mut right) = (0, side.rows.len());
        while left < right {
            control.check()?;
            account.work(1);
            let middle = left + (right - left) / 2;
            if side.rows[side.row_position(middle)].ordinal < sample.ordinal {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        for position in left..side.rows.len() {
            control.check()?;
            account.work(1);
            let position = side.row_position(position);
            let row = &side.rows[position];
            if row.ordinal != sample.ordinal {
                break;
            }
            if binary_labels::same(&row.labels, &sample.labels, control, account)? {
                return Ok(position);
            }
        }
        Err(EvaluationError::Invalid(
            "binary child replay row is missing",
        ))
    }
}

#[derive(Clone, Copy)]
struct Active {
    row: usize,
    kind: u8,
    reserved: bool,
}
#[derive(Clone, Copy)]
struct Operand {
    side: usize,
    row: usize,
    kind: u8,
    fill: Option<f64>,
}
#[derive(Clone, Copy)]
enum Choice {
    Set(Operand),
    Pair {
        left: Operand,
        right: Operand,
        signature: usize,
    },
}
#[derive(Clone, Copy)]
enum Phase {
    StartLeft,
    Left,
    StartRight,
    Right,
    Begin,
    Pairs,
    ReadLeft,
    WaitLeft,
    ReadRight,
    WaitRight,
    Emit,
    Finished,
}
pub(crate) enum Action<'a> {
    Child {
        side: usize,
        ordinal: Option<&'a usize>,
    },
    Sample(EvaluatedSample<'a>),
    Done(Option<Diagnostic>),
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    matching: &'a VectorMatching,
    operator: TokenKind,
    return_bool: bool,
    active: [AccountedVec<Active>; 2],
    active_index: [SignatureIndex; 2],
    duplicate_pairs: [Option<(usize, usize)>; 2],
    reservations: AccountedVec<(usize, u64)>,
    reservation_index: PositionIndex,
    seen: AccountedVec<u64>,
    seen_index: PositionIndex,
    duplicate: bool,
    frontier: bool,
    phase: Phase,
    position: usize,
    retry: bool,
    choice: Option<Choice>,
    values: [Option<crate::statistics::ValueParts>; 2],
    last_diagnostic: Option<Diagnostic>,
    pending: Option<AccountedBox<CompletedStep>>,
}
impl<'a> Step<'a> {
    pub(crate) fn repeated_singleton_side(&self) -> Option<usize> {
        if self.operator.is_set()
            || self.matching.fill_left.is_some()
            || self.matching.fill_right.is_some()
        {
            return None;
        }
        let one = match self.matching.cardinality {
            Cardinality::OneToMany => 0,
            Cardinality::ManyToOne => 1,
            _ => return None,
        };
        if self.plan.sides[one].rows.len() != 1 || self.plan.sides[1 - one].rows.len() <= 1 {
            return None;
        }
        let side = match self.phase {
            Phase::Left => 0,
            Phase::Right => 1,
            Phase::WaitLeft | Phase::WaitRight => {
                let Choice::Pair { left, right, .. } = self.choice? else {
                    return None;
                };
                let operand = if matches!(self.phase, Phase::WaitLeft) {
                    left
                } else {
                    right
                };
                if operand.fill.is_some() {
                    return None;
                }
                operand.side
            }
            _ => return None,
        };
        (side == one).then_some(one)
    }

    pub(crate) fn has_set_discovery(&self) -> bool {
        self.plan.has_set_discovery()
    }
    pub(crate) fn has_pair_discovery(&self) -> bool {
        self.plan.has_pair_discovery()
    }
    pub(crate) fn new(
        plan: &'a Plan,
        matching: &'a VectorMatching,
        operator: TokenKind,
        return_bool: bool,
        account: &QueryAccounting,
    ) -> Self {
        Self {
            plan,
            matching,
            operator,
            return_bool,
            active: [
                AccountedVec::new(account, MemoryKind::Workspace),
                AccountedVec::new(account, MemoryKind::Workspace),
            ],
            active_index: [SignatureIndex::new(account), SignatureIndex::new(account)],
            duplicate_pairs: [None, None],
            reservations: AccountedVec::new(account, MemoryKind::Workspace),
            reservation_index: PositionIndex::new(account),
            seen: AccountedVec::new(account, MemoryKind::Workspace),
            seen_index: PositionIndex::new(account),
            duplicate: false,
            frontier: false,
            phase: Phase::StartLeft,
            position: 0,
            retry: false,
            choice: None,
            values: [None, None],
            last_diagnostic: None,
            pending: None,
        }
    }
    pub(crate) fn enable_replay(
        &mut self,
        target: usize,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let Some(replay) = &self.plan.replay else {
            return Ok(());
        };
        let ReplayRows::Pairs(pairs) = &replay.rows else {
            return Ok(());
        };
        let Some(pair) = pairs.get(target).copied() else {
            return Ok(());
        };
        if pair.contains(&usize::MAX) {
            return Ok(());
        }
        control.check()?;
        let present = replay
            .completed
            .borrow()
            .as_ref()
            .filter(|step| step.key == key)
            .map(|step| step.members[target / 64] & (1 << (target % 64)) != 0);
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        if let Some(present) = present {
            self.phase = if present {
                Phase::Begin
            } else {
                Phase::Finished
            };
            if present {
                for (side, row) in pair.into_iter().enumerate() {
                    control.check()?;
                    account.work(1);
                    self.active[side] = AccountedVec::with_capacity_in(1, Some(&context));
                    self.active[side].push(Active {
                        row,
                        kind: 1,
                        reserved: false,
                    });
                }
            }
        } else {
            // A query keeps only one completed scope, with no overlap during replacement.
            replay.completed.borrow_mut().take();
            let words = pairs.len() / 64 + usize::from(pairs.len() % 64 != 0);
            let mut members = AccountedVec::with_capacity_in(words, Some(&context));
            for _ in 0..words {
                control.check()?;
                account.work(1);
                members.push(0);
            }
            self.pending = Some(AccountedBox::new(CompletedStep { key, members }, &context));
        }
        Ok(())
    }
    pub(crate) fn enable_set_discovery(
        &mut self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if !self.operator.is_set() || !self.plan.has_set_discovery() {
            return Ok(());
        }
        self.enable_input_discovery(key, control, account)
    }
    pub(crate) fn enable_pair_discovery(
        &mut self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.operator.is_set() || !self.plan.has_pair_discovery() {
            return Ok(());
        }
        self.enable_input_discovery(key, control, account)
    }
    fn enable_input_discovery(
        &mut self,
        key: ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let replay = self.plan.replay.as_ref().expect("input discovery plan");
        control.check()?;
        let completed = replay.completed.borrow();
        if let Some(step) = completed.as_ref().filter(|step| step.key == key) {
            if let ReplayRows::PairDiscovery(plan) = &replay.rows
                && plan.has_frontier()
                && !self.operator.is_set()
                && self.matching.fill_left.is_none()
                && self.matching.fill_right.is_none()
            {
                control.check()?;
                self.frontier = true;
                self.phase = Phase::Pairs;
                return Ok(());
            }
            let left = self.plan.sides[0].rows.len();
            let rows = left + self.plan.sides[1].rows.len();
            // Ascending bits retain physical row order across the side boundary.
            for (index, &members) in step.members.iter().enumerate() {
                control.check()?;
                account.work(1);
                let base = index * 64;
                let remaining = rows - base;
                let mut bits = if remaining < 64 {
                    members & ((1u64 << remaining) - 1)
                } else {
                    members
                };
                while bits != 0 {
                    control.check()?;
                    account.work(1);
                    let position = base + bits.trailing_zeros() as usize;
                    let (side, row) = if position < left {
                        (0, position)
                    } else {
                        (1, position - left)
                    };
                    self.active[side].push(Active {
                        row,
                        kind: 1,
                        reserved: false,
                    });
                    bits &= bits - 1;
                }
            }
            control.check()?;
            self.phase = Phase::Begin;
            return Ok(());
        }
        drop(completed);
        // Only a fully exhausted discovery pass can be reused by another output row.
        replay.completed.borrow_mut().take();
        match &replay.rows {
            ReplayRows::SetDiscovery { prefix } => prefix.set(None),
            ReplayRows::PairDiscovery(plan) => plan.reset(control, account)?,
            ReplayRows::Pairs(_) | ReplayRows::PairCandidate => {
                unreachable!("input discovery plan")
            }
        }
        let rows = self.plan.sides[0].rows.len() + self.plan.sides[1].rows.len();
        let words = rows.div_ceil(64);
        let context = AllocationContext::new(account, MemoryKind::Workspace);
        let mut members = AccountedVec::with_capacity_in(words, Some(&context));
        for _ in 0..words {
            control.check()?;
            account.work(1);
            members.push(0);
        }
        self.pending = Some(AccountedBox::new(CompletedStep { key, members }, &context));
        Ok(())
    }
    pub(crate) fn complete_replay(&mut self, control: &Control) -> Result<(), Interrupted> {
        if self.pending.is_some() {
            control.check()?;
            let completed = self
                .pending
                .take()
                .expect("completed binary replay candidate");
            let replay = self
                .plan
                .replay
                .as_ref()
                .expect("certified complete binary replay");
            if let ReplayRows::SetDiscovery { prefix } = &replay.rows {
                prefix.set(None);
            }
            *replay.completed.borrow_mut() = Some(completed);
        }
        Ok(())
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample<'_>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        match self.phase {
            Phase::Left | Phase::Right => {
                if sample.value.kind_bit() != 1 {
                    self.pending = None;
                }
                let side = usize::from(matches!(self.phase, Phase::Right));
                let row = self.plan.position(side, &sample, control, account)?;
                if (self.operator.is_set() || self.plan.has_pair_discovery())
                    && self.pending.is_some()
                {
                    if self.active[side].last().is_some_and(|last| last.row >= row) {
                        self.pending = None;
                    } else {
                        let position = row
                            + if side == 0 {
                                0
                            } else {
                                self.plan.sides[0].rows.len()
                            };
                        control.check()?;
                        account.work(1);
                        let pending = self.pending.as_mut().expect("input discovery candidate");
                        pending.members[position / 64] |= 1 << (position % 64);
                    }
                }
                self.active[side].push(Active {
                    row,
                    kind: sample.value.kind_bit(),
                    reserved: false,
                });
            }
            Phase::WaitLeft | Phase::WaitRight => {
                let index = usize::from(matches!(self.phase, Phase::WaitRight));
                let operand = self.operand(index).expect("pending pair operand");
                if self.values[index].is_none() && sample.value.kind_bit() == operand.kind {
                    self.values[index] = Some(sample.into_parts());
                }
            }
            _ => unreachable!("binary phase consumes child"),
        }
        Ok(())
    }
    fn signature(&self, side: usize, active: Active) -> usize {
        self.plan.sides[side].signatures[active.row]
    }
    fn operand(&self, index: usize) -> Option<Operand> {
        match self.choice.expect("current binary choice") {
            Choice::Set(value) => (index == 0).then_some(value),
            Choice::Pair { left, right, .. } => Some(if index == 0 { left } else { right }),
        }
    }
    fn swapped(&self) -> bool {
        self.matching.cardinality == Cardinality::OneToMany
    }
    fn set_ordinal(&self, operand: Operand) -> usize {
        operand.row
            + if operand.side == 0 {
                0
            } else {
                self.plan.sides[0].rows.len()
            }
    }
    fn record_seen(
        &mut self,
        hash: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if self.frontier {
            control.check()?;
            account.work(1);
            return Ok(());
        }
        if self
            .seen_index
            .find(
                hash,
                |position| Ok(self.seen[position] == hash),
                control,
                account,
            )?
            .is_some()
        {
            self.duplicate = true;
        } else {
            let position = self.seen.len();
            self.seen.push(hash);
            self.seen_index.insert_new(
                hash,
                position,
                |prior| Ok(self.seen[prior]),
                control,
                account,
            )?;
        }
        Ok(())
    }
    fn skip_set_prefix(
        &mut self,
        progress: Option<Progress<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, EvaluationError> {
        let Some(progress) = progress else {
            return Ok(false);
        };
        let Some(target) = progress.target else {
            return Ok(false);
        };
        let Some(Choice::Set(operand)) = self.choice else {
            return Ok(false);
        };
        let ordinal = self.set_ordinal(operand);
        if ordinal == target {
            return Ok(false);
        }
        control.check()?;
        account.work(1);
        if self
            .plan
            .set_prefix(progress.key)
            .is_none_or(|prefix| ordinal > prefix)
        {
            return Ok(false);
        }
        // The successful prefix validated these exact immutable descriptor labels.
        let hash = self.plan.sides[operand.side].rows[operand.row].hash;
        self.record_seen(hash, control, account)?;
        control.check()?;
        self.phase = Phase::Pairs;
        Ok(true)
    }
    fn skip_pair_prefix(
        &mut self,
        progress: Option<Progress<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, EvaluationError> {
        if self.frontier {
            return Ok(false);
        }
        let Some(progress) = progress else {
            return Ok(false);
        };
        let Some(target) = progress.target else {
            return Ok(false);
        };
        let Some(plan) = self.plan.pair_discovery(progress.key) else {
            return Ok(false);
        };
        let Some(Choice::Pair {
            left,
            right,
            signature,
        }) = self.choice
        else {
            return Ok(false);
        };
        let mut rows = [0; 2];
        rows[left.side] = left.row;
        rows[right.side] = right.row;
        let Some(ordinal) = plan.ordinal(rows, usize::from(self.swapped()), control, account)?
        else {
            return Ok(false);
        };
        if ordinal == target {
            return Ok(false);
        }
        let Some(kept) = plan.state(ordinal, control, account)? else {
            return Ok(false);
        };
        let hash = if plan.has_frontier() {
            label_hash::full(&self.plan.cache.entries[ordinal].labels, control, account)?
        } else {
            plan.hash(ordinal)
        };
        self.reserve_pair(signature, hash, control, account)?;
        if kept {
            self.record_seen(hash, control, account)?;
        }
        control.check()?;
        self.phase = Phase::Pairs;
        Ok(true)
    }
    fn record_pair_prefix(
        &self,
        progress: Option<Progress<'_>>,
        ordinal: usize,
        kept: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        if let Some(progress) = progress
            && let Some(plan) = self.plan.pair_discovery(progress.key)
        {
            plan.record(ordinal, kept, control, account)?;
        }
        Ok(())
    }
    fn labels(
        &self,
        operand: Operand,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<SampleLabels<'a>, Interrupted> {
        let labels = &self.plan.sides[operand.side].rows[operand.row].labels;
        Ok(if operand.fill.is_some() {
            SampleLabels::Owned(binary_labels::matched(
                labels,
                self.matching,
                control,
                account,
            )?)
        } else {
            SampleLabels::Borrowed(labels)
        })
    }
    fn result_labels(
        &self,
        cache: &mut Cache,
        key: AccountedVec<u8>,
        operands: [&OwnedLabels; 2],
        drop_name: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(usize, OwnedLabels), EvaluationError> {
        let hash = label_hash::bytes(&key, control, account)?;
        if let Some(position) = cache.position(&key, hash, control, account)? {
            return Ok((
                position,
                cache.entries[position].labels.copy_checked(
                    account,
                    MemoryKind::Workspace,
                    control,
                )?,
            ));
        }
        let labels = binary_labels::projected(
            operands[0],
            operands[1],
            self.operator,
            self.matching,
            drop_name,
            control,
            account,
        )?;
        let copy = labels.copy_checked(account, MemoryKind::Workspace, control)?;
        let position = cache.entries.len();
        cache.entries.push(CacheEntry {
            key: Key { bytes: key, hash },
            labels,
        });
        cache.index.insert_new(
            hash,
            position,
            |prior| Ok(cache.entries[prior].key.hash),
            control,
            account,
        )?;
        Ok((position, copy))
    }

    fn prepare_active(
        &mut self,
        side: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        let active = &self.active[side];
        let signatures = &self.plan.sides[side].signatures;
        let index = &mut self.active_index[side];
        for position in 0..active.len() {
            control.check()?;
            account.work(1);
            let signature = signatures[active[position].row];
            let key_of = |prior: usize| signatures[active[prior].row];
            if let Some(prior) = index.find(signature, key_of, control, account)? {
                self.duplicate_pairs[side].get_or_insert((position, prior));
            } else {
                index.insert_new(
                    signature,
                    position,
                    self.plan.signature_count,
                    key_of,
                    control,
                    account,
                )?;
            }
        }
        Ok(())
    }

    fn first_active(
        &self,
        side: usize,
        signature: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<usize>, Interrupted> {
        self.active_index[side].find(
            signature,
            |position| self.signature(side, self.active[side][position]),
            control,
            account,
        )
    }

    fn reservation_position(
        &self,
        signature: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(usize, usize), Interrupted> {
        let many = usize::from(self.swapped());
        if let Some(position) = self.first_active(many, signature, control, account)? {
            return Ok((many, position));
        }
        Ok((
            1 - many,
            self.first_active(1 - many, signature, control, account)?
                .expect("chosen signature has an active row"),
        ))
    }

    fn reserve_pair(
        &mut self,
        signature: usize,
        hash: u64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if self.frontier {
            control.check()?;
            account.work(1);
            return Ok(());
        }
        let (reserved_side, reserved_position) =
            self.reservation_position(signature, control, account)?;
        if self.matching.cardinality == Cardinality::OneToOne {
            if self.active[reserved_side][reserved_position].reserved {
                return Err(EvaluationError::Invalid(
                    "multiple matches for labels: many-to-one matching must be explicit (group_left/group_right)",
                ));
            }
        } else {
            let key_hash = reservation_hash(signature, hash);
            if self
                .reservation_index
                .find(
                    key_hash,
                    |position| Ok(self.reservations[position] == (signature, hash)),
                    control,
                    account,
                )?
                .is_some()
            {
                return Err(EvaluationError::Invalid(
                    "multiple matches for labels: grouping labels must ensure unique matches",
                ));
            }
            let position = self.reservations.len();
            self.reservations.push((signature, hash));
            self.reservation_index.insert_new(
                key_hash,
                position,
                |prior| {
                    let (signature, hash) = self.reservations[prior];
                    Ok(reservation_hash(signature, hash))
                },
                control,
                account,
            )?;
        }
        self.active[reserved_side][reserved_position].reserved = true;
        Ok(())
    }
    fn reserved(
        &self,
        signature: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        let (side, position) = self.reservation_position(signature, control, account)?;
        Ok(self.active[side][position].reserved)
    }
    fn validate(
        &self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        if self.operator.is_set() {
            return Ok(());
        }
        let side = usize::from(!self.swapped());
        if let Some((position, previous)) = self.duplicate_pairs[side] {
            let active = &self.active[side][position];
            let previous = &self.active[side][previous];
            control.check()?;
            account.work(1);
            let row = &self.plan.sides[side].rows[active.row].labels;
            let prior = &self.plan.sides[side].rows[previous.row].labels;
            let group = binary_labels::matched(row, self.matching, control, account)?;
            let group = binary_labels::render(&group, control, account)?;
            let mut a = binary_labels::render(row, control, account)?;
            let mut b = binary_labels::render(prior, control, account)?;
            if binary_labels::compare(&a, &b, control, account)?.is_gt() {
                std::mem::swap(&mut a, &mut b);
            }
            let mut message = AccountedVec::new(account, MemoryKind::Workspace);
            let side_name = if self.swapped() {
                &b"left"[..]
            } else {
                &b"right"[..]
            };
            for bytes in [
                &b"found duplicate series for the match group "[..],
                &group,
                b" on the ",
                side_name,
                b" hand-side of the operation: [",
                &a,
                b", ",
                &b,
                b"];many-to-many matching not allowed: matching labels must be unique on one side",
            ] {
                append_bytes_checked(&mut message, bytes, account, control)?;
            }
            return Err(EvaluationError::Runtime(message));
        }
        Ok(())
    }
    fn present(
        &self,
        side: usize,
        signature: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        Ok(self
            .first_active(side, signature, control, account)?
            .is_some())
    }
    fn choose(
        &mut self,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        loop {
            control.check()?;
            account.work(1);
            if self.operator.is_set() {
                let side = usize::from(self.retry);
                let Some(active) = self.active[side].get(self.position).copied() else {
                    if self.operator == TokenKind::LOR && !self.retry {
                        self.retry = true;
                        self.position = 0;
                        continue;
                    }
                    return Ok(false);
                };
                self.position += 1;
                let signature = self.signature(side, active);
                let present = if self.operator == TokenKind::LOR && !self.retry {
                    false
                } else {
                    self.present(1 - side, signature, control, account)?
                };
                let keep = match self.operator {
                    TokenKind::LAND => present,
                    TokenKind::LUNLESS => !present,
                    TokenKind::LOR => !self.retry || !present,
                    _ => unreachable!(),
                };
                if !keep {
                    continue;
                }
                self.choice = Some(Choice::Set(Operand {
                    side,
                    row: active.row,
                    kind: active.kind,
                    fill: None,
                }));
                return Ok(true);
            }
            let many = usize::from(self.swapped());
            let one = 1 - many;
            let side = if self.retry { one } else { many };
            let Some(active) = self.active[side].get(self.position).copied() else {
                if !self.retry && self.matching.fill_left.is_some() {
                    self.retry = true;
                    self.position = 0;
                    continue;
                }
                return Ok(false);
            };
            self.position += 1;
            let signature = self.signature(side, active);
            let own = Operand {
                side,
                row: active.row,
                kind: active.kind,
                fill: None,
            };
            let (left, right) = if self.retry {
                if self.reserved(signature, control, account)? {
                    continue;
                }
                let fake = Operand {
                    fill: self.matching.fill_left,
                    kind: 1,
                    ..own
                };
                (fake, own)
            } else {
                let matched = self
                    .first_active(one, signature, control, account)?
                    .map(|position| self.active[one][position]);
                let right = if let Some(other) = matched {
                    Operand {
                        side: one,
                        row: other.row,
                        kind: other.kind,
                        fill: None,
                    }
                } else if let Some(fill) = self.matching.fill_right {
                    Operand {
                        fill: Some(fill),
                        kind: 1,
                        ..own
                    }
                } else {
                    continue;
                };
                (own, right)
            };
            self.choice = Some(Choice::Pair {
                left,
                right,
                signature,
            });
            return Ok(true);
        }
    }
    fn choose_frontier(
        &mut self,
        progress: Option<Progress<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        let replay = self.plan.replay.as_ref().expect("pair frontier owner");
        let ReplayRows::PairDiscovery(plan) = &replay.rows else {
            unreachable!("pair frontier owner")
        };
        let completed = replay.completed.borrow();
        let completed = completed.as_ref().expect("completed pair inputs");
        let many = usize::from(self.swapped());
        let inputs = crate::pair_discovery::Inputs {
            members: &completed.members,
            left_rows: self.plan.sides[0].rows.len(),
            many,
        };
        let target = progress
            .filter(|progress| *progress.key == completed.key)
            .and_then(|progress| progress.target);
        let Some(pair) = plan.next(self.position, target, &inputs, control, account)? else {
            return Ok(false);
        };
        self.position = pair[many] + 1;
        let operands = [many, 1 - many].map(|side| Operand {
            side,
            row: pair[side],
            kind: 1,
            fill: None,
        });
        self.choice = Some(Choice::Pair {
            left: operands[0],
            right: operands[1],
            signature: self.plan.sides[many].signatures[pair[many]],
        });
        Ok(true)
    }

    pub(crate) fn advance(
        &mut self,
        node: NodeId,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
        progress: Option<Progress<'_>>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        loop {
            control.check()?;
            account.work(1);
            match self.phase {
                Phase::StartLeft => {
                    self.phase = Phase::Left;
                    return Ok(Action::Child {
                        side: 0,
                        ordinal: None,
                    });
                }
                Phase::Left => self.phase = Phase::StartRight,
                Phase::StartRight => {
                    self.phase = Phase::Right;
                    return Ok(Action::Child {
                        side: 1,
                        ordinal: None,
                    });
                }
                Phase::Right => {
                    if self.operator.is_set() || self.plan.has_pair_discovery() {
                        self.complete_replay(control)?;
                    }
                    self.phase = Phase::Begin;
                }
                Phase::Begin => {
                    let empty = self.active[0].is_empty() || self.active[1].is_empty();
                    if !self.operator.is_set()
                        && empty
                        && (self.active[0].is_empty() && self.active[1].is_empty()
                            || self.matching.fill_left.is_none()
                                && self.matching.fill_right.is_none())
                    {
                        self.phase = Phase::Finished;
                        continue;
                    }
                    if self.operator.is_set() {
                        let side = usize::from(self.operator != TokenKind::LOR);
                        if !self.active[1 - side].is_empty() {
                            self.prepare_active(side, control, account)?;
                        }
                    } else {
                        let one = usize::from(!self.swapped());
                        self.prepare_active(one, control, account)?;
                        self.validate(control, account)?;
                        self.prepare_active(1 - one, control, account)?;
                    }
                    self.phase = Phase::Pairs;
                }
                Phase::Pairs => {
                    let chosen = if self.frontier {
                        self.choose_frontier(progress, control, account)?
                    } else {
                        self.choose(control, account)?
                    };
                    self.phase = if chosen {
                        Phase::ReadLeft
                    } else {
                        Phase::Finished
                    };
                }
                Phase::ReadLeft | Phase::ReadRight => {
                    let index = usize::from(matches!(self.phase, Phase::ReadRight));
                    if index == 0
                        && (self.skip_set_prefix(progress, control, account)?
                            || self.skip_pair_prefix(progress, control, account)?)
                    {
                        continue;
                    }
                    let Some(operand) = self.operand(index) else {
                        self.phase = Phase::Emit;
                        continue;
                    };
                    if let Some(value) = operand.fill {
                        self.values[index] = Some(crate::statistics::ValueParts::new(
                            OwnedValue::Float(value),
                            account,
                        ));
                        self.phase = if index == 0 {
                            Phase::ReadRight
                        } else {
                            Phase::Emit
                        };
                    } else {
                        self.phase = if index == 0 {
                            Phase::WaitLeft
                        } else {
                            Phase::WaitRight
                        };
                        return Ok(Action::Child {
                            side: operand.side,
                            ordinal: Some(&self.plan.sides[operand.side].rows[operand.row].ordinal),
                        });
                    }
                }
                Phase::WaitLeft | Phase::WaitRight => {
                    let index = usize::from(matches!(self.phase, Phase::WaitRight));
                    if self.values[index].is_none() {
                        return Err(EvaluationError::Invalid("binary operand replay is empty"));
                    }
                    self.phase = if index == 0 {
                        Phase::ReadRight
                    } else {
                        Phase::Emit
                    };
                }
                Phase::Emit => {
                    self.phase = Phase::Pairs;
                    let value = self.values[0].take().expect("first replay value");
                    let (ordinal, labels, parts, drop_name) = match self
                        .choice
                        .expect("binary choice")
                    {
                        Choice::Set(operand) => {
                            let descriptor = &self.plan.sides[operand.side].rows[operand.row];
                            control.check()?;
                            (
                                self.set_ordinal(operand),
                                SampleLabels::Borrowed(&descriptor.labels),
                                value,
                                descriptor.drop_name,
                            )
                        }
                        Choice::Pair {
                            left,
                            right,
                            signature,
                        } => {
                            let other = self.values[1].take().expect("second replay value");
                            let (lhs, rhs) = if self.swapped() {
                                (other, value)
                            } else {
                                (value, other)
                            };
                            let pair = binary_value::evaluate(
                                self.operator,
                                lhs.value,
                                rhs.value,
                                control,
                                account,
                            )?;
                            if pair.diagnostic.is_some() {
                                self.last_diagnostic = pair.diagnostic;
                            }
                            let Some(mut value) = pair.value else {
                                continue;
                            };
                            let left = self.labels(left, control, account)?;
                            let right = self.labels(right, control, account)?;
                            let mut key = AccountedVec::new(account, MemoryKind::Workspace);
                            binary_labels::append(&mut key, left.as_ref(), None, control, account)?;
                            binary_labels::append(
                                &mut key,
                                right.as_ref(),
                                None,
                                control,
                                account,
                            )?;
                            let (ordinal, labels) = if let Some(preparation) =
                                preparation.filter(|owner| owner.node == node)
                            {
                                let mut cache = preparation.cache.borrow_mut();
                                self.result_labels(
                                    &mut cache,
                                    key,
                                    [left.as_ref(), right.as_ref()],
                                    !options.delayed_name_removal && self.return_bool,
                                    control,
                                    account,
                                )
                                .map(|(ordinal, labels)| (ordinal, SampleLabels::Owned(labels)))?
                            } else {
                                frozen_labels(&self.plan.cache, &key, control, account)?
                            };
                            let hash = label_hash::full(&labels, control, account)?;
                            self.reserve_pair(signature, hash, control, account)?;
                            if self.return_bool {
                                value = OwnedValue::Float(f64::from(pair.keep));
                            } else if !pair.keep {
                                self.record_pair_prefix(
                                    progress, ordinal, false, control, account,
                                )?;
                                continue;
                            }
                            drop((lhs.float_samples, rhs.float_samples));
                            (
                                ordinal,
                                labels,
                                crate::statistics::ValueParts::new(value, account),
                                self.return_bool,
                            )
                        }
                    };
                    let hash = label_hash::full(&labels, control, account)?;
                    self.record_seen(hash, control, account)?;
                    if !self.operator.is_set() {
                        self.record_pair_prefix(progress, ordinal, true, control, account)?;
                    }
                    if self.operator.is_set()
                        && let Some(progress) = progress
                    {
                        self.plan
                            .extend_set_prefix(progress.key, ordinal, control, account)?;
                    }
                    if !self.operator.is_set()
                        && let Some(pending) = &mut self.pending
                    {
                        control.check()?;
                        account.work(1);
                        pending.members[ordinal / 64] |= 1 << (ordinal % 64);
                    }
                    return Ok(Action::Sample(EvaluatedSample {
                        ordinal,
                        labels,
                        float_samples: parts.float_samples,
                        value: parts.value,
                        drop_name,
                        source_time_ms: options.time_ms,
                        source_start_time_ms: 0,
                    }));
                }
                Phase::Finished => {
                    if self.duplicate
                        && (!options.delayed_name_removal || options.start_ms != options.end_ms)
                    {
                        return Err(EvaluationError::Invalid(
                            "vector cannot contain metrics with the same labelset",
                        ));
                    }
                    return Ok(Action::Done(self.last_diagnostic));
                }
            }
        }
    }
}
fn reservation_hash(signature: usize, hash: u64) -> u64 {
    label_hash::avalanche(signature as u64) ^ hash.rotate_left(17)
}
fn frozen_labels<'a>(
    cache: &'a Cache,
    key: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(usize, SampleLabels<'a>), EvaluationError> {
    let hash = label_hash::bytes(key, control, account)?;
    if let Some(position) = cache.position(key, hash, control, account)? {
        control.check()?;
        return Ok((
            position,
            SampleLabels::Borrowed(&cache.entries[position].labels),
        ));
    }
    Err(EvaluationError::Invalid(
        "frozen binary label cache key is missing",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{accounting::EngineAccounting, storage::LabelRef};

    fn labels(value: &[u8], account: &QueryAccounting) -> OwnedLabels {
        OwnedLabels::copy_from(
            &[LabelRef { name: b"id", value }],
            account,
            MemoryKind::Workspace,
        )
    }

    fn discovery_options() -> EvaluationOptions {
        EvaluationOptions {
            time_ms: 1000,
            start_ms: 1000,
            end_ms: 3000,
            step_ms: 1000,
            lookback_ms: 300_000,
            start_timestamps: false,
            delayed_name_removal: false,
        }
    }
    fn discovery_key() -> ReplayKey {
        ReplayKey::new(
            discovery_options(),
            0,
            300_000_000_000,
            crate::storage::TimeBounds {
                start_ms: 0,
                end_ms: 3000,
            },
        )
    }
    fn discovery_plan(
        count: usize,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        discovery_plan_names(count, false, matching, control, account)
    }
    fn discovery_plan_names(
        count: usize,
        disjoint_right: bool,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        discovery_plan_shape([count, count], disjoint_right, matching, control, account)
    }
    fn discovery_plan_shape(
        counts: [usize; 2],
        disjoint_right: bool,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        let sides = [0, 1].map(|side| {
            let count = counts[side];
            let mut rows = AccountedVec::new(account, MemoryKind::Workspace);
            for position in 0..count {
                let value = if disjoint_right && side == 1 {
                    format!("r{position:08}")
                } else {
                    format!("{position:08}")
                };
                let labels = labels(value.as_bytes(), account);
                let hash = label_hash::full(&labels, control, account).unwrap();
                rows.push(Descriptor {
                    labels,
                    ordinal: (count - position) * (side + 2),
                    hash,
                    drop_name: false,
                    first_order: position,
                    sources: 0..0,
                    kinds: 1,
                    heads: Default::default(),
                });
            }
            rows
        });
        let [left, right] = sides;
        Plan::new(left, right, matching, control, account).unwrap()
    }
    fn discovery_sample(
        plan: &Plan,
        side: usize,
        row: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> EvaluatedSample<'static> {
        let descriptor = &plan.sides[side].rows[row];
        EvaluatedSample {
            ordinal: descriptor.ordinal,
            labels: descriptor
                .labels
                .copy_checked(account, MemoryKind::Workspace, control)
                .unwrap()
                .into(),
            value: OwnedValue::Float((1000 * side + row + 1) as f64),
            float_samples: None,
            source_time_ms: 1000,
            source_start_time_ms: 0,
            drop_name: false,
        }
    }
    #[derive(Debug, PartialEq, Eq)]
    struct DiscoveryTrace {
        scans: usize,
        rereads: Vec<(usize, usize)>,
        outputs: Vec<(usize, Vec<u8>, u64)>,
    }
    fn discovery_trace(
        step: &mut Step<'_>,
        members: [&[usize]; 2],
        control: &Control,
        account: &QueryAccounting,
    ) -> DiscoveryTrace {
        discovery_trace_scoped(step, members, None, None, control, account)
    }
    fn discovery_trace_scoped(
        step: &mut Step<'_>,
        members: [&[usize]; 2],
        progress: Option<Progress<'_>>,
        stop_at: Option<usize>,
        control: &Control,
        account: &QueryAccounting,
    ) -> DiscoveryTrace {
        let mut trace = DiscoveryTrace {
            scans: 0,
            rereads: Vec::new(),
            outputs: Vec::new(),
        };
        loop {
            match step
                .advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    progress,
                    control,
                    account,
                )
                .unwrap()
            {
                Action::Child { side, ordinal } => {
                    if let Some(ordinal) = ordinal {
                        let row = step.plan.sides[side]
                            .rows
                            .iter()
                            .position(|row| row.ordinal == *ordinal)
                            .unwrap();
                        trace.rereads.push((side, row));
                        let sample = discovery_sample(step.plan, side, row, control, account);
                        step.receive(sample, control, account).unwrap();
                    } else {
                        trace.scans += 1;
                        for row in members[side] {
                            let sample = discovery_sample(step.plan, side, *row, control, account);
                            step.receive(sample, control, account).unwrap();
                        }
                    }
                }
                Action::Sample(sample) => {
                    let OwnedValue::Float(value) = sample.value else {
                        panic!("float discovery trace")
                    };
                    trace.outputs.push((
                        sample.ordinal,
                        sample.labels.0[0].value.to_vec(),
                        value.to_bits(),
                    ));
                    if stop_at == Some(sample.ordinal) {
                        return trace;
                    }
                }
                Action::Done(diagnostic) => {
                    assert!(diagnostic.is_none());
                    step.complete_replay(control).unwrap();
                    return trace;
                }
            }
        }
    }

    fn grouped_pair_plan(
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        grouped_pair_plan_rows(3, matching, control, account)
    }

    fn grouped_pair_plan_rows(
        rows: usize,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        let many = usize::from(matching.cardinality == Cardinality::OneToMany);
        let counts = if many == 0 { [rows, 1] } else { [1, rows] };
        let mut plan = discovery_plan_shape(counts, true, matching, control, account);
        for row in 0..3 {
            let mut key = AccountedVec::new(account, MemoryKind::Workspace);
            for side in [many, 1 - many] {
                let position = if side == many { row } else { 0 };
                binary_labels::append(
                    &mut key,
                    &plan.sides[side].rows[position].labels,
                    None,
                    control,
                    account,
                )
                .unwrap();
            }
            let hash = label_hash::bytes(&key, control, account).unwrap();
            plan.cache.entries.push(CacheEntry {
                key: Key { bytes: key, hash },
                labels: labels(format!("output{row}").as_bytes(), account),
            });
            plan.cache
                .index
                .insert_new(
                    hash,
                    row,
                    |prior| Ok(plan.cache.entries[prior].key.hash),
                    control,
                    account,
                )
                .unwrap();
        }
        plan
    }

    fn prefix_pair_plan(
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        prefix_pair_plan_rows(3, matching, control, account)
    }

    fn prefix_pair_plan_rows(
        rows: usize,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Plan {
        let mut plan = grouped_pair_plan_rows(rows, matching, control, account);
        plan.certify_replay(&[0, 1, 2], matching.cardinality, control, account)
            .unwrap();
        plan.certify_pair_discovery(matching.cardinality, control, account)
            .unwrap();
        assert!(plan.has_pair_discovery());
        plan
    }

    #[test]
    fn pair_prefix_preserves_selected_unvisited_and_backward_reads() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching {
                cardinality,
                on: true,
                ..VectorMatching::default()
            };
            let plan = prefix_pair_plan(&matching, &control, &account);
            let many = usize::from(cardinality == Cardinality::OneToMany);
            let members = if many == 0 {
                [&[0, 1, 2][..], &[0][..]]
            } else {
                [&[0][..], &[0, 1, 2][..]]
            };
            let key = discovery_key();
            for (target, rows, outputs) in [
                (1, vec![0, 1], vec![0, 1]),
                (2, vec![2], vec![2]),
                (0, vec![0], vec![0]),
                (99, vec![], vec![]),
            ] {
                let cold = plan.pair_discovery(&key).is_none();
                let mut step = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                step.enable_pair_discovery(key, &control, &account).unwrap();
                let trace = discovery_trace_scoped(
                    &mut step,
                    members,
                    Some(Progress {
                        key: &key,
                        target: Some(target),
                    }),
                    Some(target),
                    &control,
                    &account,
                );
                assert_eq!(trace.scans, 2 * usize::from(cold));
                assert_eq!(step.frontier, !cold);
                if !cold {
                    assert!(step.active.iter().all(|rows| rows.is_empty()));
                    assert!(step.reservations.is_empty());
                    assert!(step.seen.is_empty());
                }
                assert_eq!(
                    trace.rereads,
                    rows.into_iter()
                        .flat_map(|row| [(many, row), (1 - many, 0)])
                        .collect::<Vec<_>>()
                );
                assert_eq!(
                    trace.outputs.iter().map(|row| row.0).collect::<Vec<_>>(),
                    outputs
                );
                if target == 99 {
                    assert!(matches!(step.phase, Phase::Finished));
                }
            }
            let mut ordinary = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            ordinary.enable_replay(0, key, &control, &account).unwrap();
            assert!(
                matches!(ordinary.phase, Phase::StartLeft),
                "a prefix is not full Done"
            );
            drop(ordinary);
            plan.clear_discovery();
            assert!(plan.pair_discovery(&key).is_none());
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn pair_prefix_suppressed_comparisons_reserve_without_seen() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            for return_bool in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let matching = VectorMatching {
                    cardinality,
                    on: true,
                    ..VectorMatching::default()
                };
                let plan = prefix_pair_plan_rows(4, &matching, &control, &account);
                let many = usize::from(cardinality == Cardinality::OneToMany);
                let members = if many == 0 {
                    [&[0, 1, 2][..], &[0][..]]
                } else {
                    [&[0][..], &[0, 1, 2][..]]
                };
                let key = discovery_key();
                for cold in [true, false] {
                    let mut step =
                        Step::new(&plan, &matching, TokenKind::GTR, return_bool, &account);
                    step.enable_pair_discovery(key, &control, &account).unwrap();
                    let trace = discovery_trace_scoped(
                        &mut step,
                        members,
                        Some(Progress {
                            key: &key,
                            target: Some(1),
                        }),
                        None,
                        &control,
                        &account,
                    );
                    assert_eq!(trace.scans, 2 * usize::from(cold));
                    assert_eq!(trace.rereads.len(), if cold { 6 } else { 2 });
                    assert_eq!(
                        trace.outputs.len(),
                        if !return_bool {
                            0
                        } else if cold {
                            3
                        } else {
                            1
                        }
                    );
                    assert_eq!(step.seen.len(), if return_bool { 3 } else { 0 });
                    assert_eq!(step.reservations.len(), 3);
                    assert!(step.active[many][0].reserved);
                    for ordinal in 0..3 {
                        assert_eq!(
                            plan.pair_discovery(&key)
                                .unwrap()
                                .state(ordinal, &control, &account)
                                .unwrap(),
                            Some(return_bool)
                        );
                    }
                }
                drop(plan);
                account.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn pair_prefix_replaces_exact_scope_and_discards_incomplete_discovery() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching {
            cardinality: Cardinality::ManyToOne,
            on: true,
            ..VectorMatching::default()
        };
        let plan = prefix_pair_plan(&matching, &control, &account);
        let base = discovery_key();
        let mut keys = vec![base];
        for field in 0..base.clock.len() {
            let mut key = base;
            key.clock[field] += 1;
            keys.push(key);
        }
        keys.extend([
            ReplayKey {
                start_timestamps: true,
                ..base
            },
            ReplayKey {
                delayed_name_removal: true,
                ..base
            },
            base,
        ]);
        for key in keys {
            let mut step = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            step.enable_pair_discovery(key, &control, &account).unwrap();
            assert!(plan.pair_discovery(&key).is_none());
            let trace = discovery_trace_scoped(
                &mut step,
                [&[0, 2], &[0]],
                Some(Progress {
                    key: &key,
                    target: Some(0),
                }),
                Some(0),
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2);
            let progress = plan.pair_discovery(&key).unwrap();
            assert_eq!(progress.state(0, &control, &account).unwrap(), Some(true));
            assert_eq!(progress.state(1, &control, &account).unwrap(), None);
            assert_eq!(progress.state(2, &control, &account).unwrap(), None);
        }
        for case in 0..4 {
            plan.clear_discovery();
            let mut step = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            step.enable_pair_discovery(base, &control, &account)
                .unwrap();
            assert!(matches!(
                step.advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    None,
                    &control,
                    &account
                )
                .unwrap(),
                Action::Child {
                    side: 0,
                    ordinal: None
                }
            ));
            for row in if case == 1 { [2, 0] } else { [0, 0] } {
                let mut sample = discovery_sample(&plan, 0, row, &control, &account);
                if case == 2 {
                    let histogram = crate::histogram::FloatHistogram::default();
                    sample.value = OwnedValue::copy_from(
                        crate::value::SampleValueRef::FloatHistogram(histogram.as_ref()),
                        &account,
                        MemoryKind::Workspace,
                    );
                }
                step.receive(sample, &control, &account).unwrap();
                if case == 3 {
                    break;
                }
            }
            assert!(plan.pair_discovery(&base).is_none());
            assert_eq!(step.pending.is_none(), case != 3);
        }
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn pair_prefix_polls_after_seen_and_before_credit_publication() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            for (publish, frontier) in [(false, false), (true, false), (true, true)] {
                let mut terminal_work = 0;
                for boundary in 0..3 {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let matching = VectorMatching {
                        cardinality,
                        on: true,
                        ..VectorMatching::default()
                    };
                    let plan = prefix_pair_plan_rows(
                        if frontier { 3 } else { 4 },
                        &matching,
                        &control,
                        &account,
                    );
                    let many = usize::from(cardinality == Cardinality::OneToMany);
                    let members = if many == 0 {
                        [&[0, 1, 2][..], &[0][..]]
                    } else {
                        [&[0][..], &[0, 1, 2][..]]
                    };
                    let key = discovery_key();
                    {
                        let mut cold = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                        cold.enable_pair_discovery(key, &control, &account).unwrap();
                        discovery_trace_scoped(
                            &mut cold,
                            members,
                            Some(Progress {
                                key: &key,
                                target: Some(if publish { 0 } else { 2 }),
                            }),
                            Some(if publish { 0 } else { 2 }),
                            &control,
                            &account,
                        );
                    }
                    let progress = Some(Progress {
                        key: &key,
                        target: Some(if publish { 1 } else { 99 }),
                    });
                    let mut step = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                    step.enable_pair_discovery(key, &control, &account).unwrap();
                    assert_eq!(step.frontier, frontier);
                    if publish {
                        loop {
                            let Action::Child {
                                side,
                                ordinal: Some(ordinal),
                            } = step
                                .advance(
                                    NodeId(0),
                                    discovery_options(),
                                    None,
                                    progress,
                                    &control,
                                    &account,
                                )
                                .unwrap()
                            else {
                                panic!("fresh pair must request both operands")
                            };
                            let row = plan.sides[side]
                                .rows
                                .iter()
                                .position(|row| row.ordinal == *ordinal)
                                .unwrap();
                            step.receive(
                                discovery_sample(&plan, side, row, &control, &account),
                                &control,
                                &account,
                            )
                            .unwrap();
                            if matches!(step.phase, Phase::WaitRight) {
                                break;
                            }
                        }
                        step.phase = Phase::Emit;
                    } else {
                        step.prepare_active(1 - many, &control, &account).unwrap();
                        step.validate(&control, &account).unwrap();
                        step.prepare_active(many, &control, &account).unwrap();
                        for row in 0..3 {
                            assert!(step.choose(&control, &account).unwrap());
                            step.phase = Phase::ReadLeft;
                            if row < 2 {
                                assert!(
                                    step.skip_pair_prefix(progress, &control, &account).unwrap()
                                );
                            }
                        }
                    }
                    let seen_before = step.seen.len();
                    let before = account.snapshot().work;
                    let mut operation = || {
                        if publish {
                            match step.advance(
                                NodeId(0),
                                discovery_options(),
                                None,
                                progress,
                                &control,
                                &account,
                            )? {
                                Action::Sample(sample) => {
                                    assert_eq!(sample.ordinal, 1);
                                    Ok(true)
                                }
                                _ => panic!("fresh pair must publish one sample"),
                            }
                        } else {
                            step.skip_pair_prefix(progress, &control, &account)
                        }
                    };
                    let result = if boundary == 0 {
                        operation()
                    } else {
                        crate::test_storage::interrupt_after_work(
                            &account,
                            &control,
                            if boundary == 1 { 1 } else { terminal_work },
                            operation,
                        )
                    };
                    if boundary == 0 {
                        assert!(matches!(result, Ok(true)));
                        terminal_work = account.snapshot().work - before;
                        assert!(terminal_work > 1);
                    } else {
                        assert!(matches!(
                            result,
                            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                        ));
                        if boundary == 2 && !frontier {
                            assert_eq!(step.seen.len(), seen_before + 1);
                        }
                        if !publish {
                            assert!(matches!(step.phase, Phase::ReadLeft));
                        }
                    }
                    let inspection = Control::default();
                    assert_eq!(
                        plan.pair_discovery(&key)
                            .unwrap()
                            .state(1, &inspection, &account)
                            .unwrap(),
                        (!publish || boundary == 0).then_some(true)
                    );
                    drop(result);
                    drop(step);
                    drop(plan);
                    account.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn pair_frontier_keeps_unscoped_warm_traversals_fresh() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching {
                cardinality,
                on: true,
                ..VectorMatching::default()
            };
            let plan = prefix_pair_plan(&matching, &control, &account);
            let many = usize::from(cardinality == Cardinality::OneToMany);
            let members = if many == 0 {
                [&[0, 1, 2][..], &[0][..]]
            } else {
                [&[0][..], &[0, 1, 2][..]]
            };
            let key = discovery_key();
            let cold = {
                let mut step = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                step.enable_pair_discovery(key, &control, &account).unwrap();
                discovery_trace_scoped(
                    &mut step,
                    members,
                    Some(Progress {
                        key: &key,
                        target: Some(2),
                    }),
                    None,
                    &control,
                    &account,
                )
            };
            for target in [None, Some(0), None] {
                let mut warm = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                warm.enable_pair_discovery(key, &control, &account).unwrap();
                assert!(warm.frontier);
                let trace = discovery_trace_scoped(
                    &mut warm,
                    [&[], &[]],
                    target.map(|target| Progress {
                        key: &key,
                        target: Some(target),
                    }),
                    target,
                    &control,
                    &account,
                );
                assert_eq!(trace.scans, 0);
                if target.is_none() {
                    assert_eq!(trace.outputs, cold.outputs);
                    assert_eq!(trace.rereads, cold.rereads);
                    assert!(matches!(warm.phase, Phase::Finished));
                } else {
                    assert_eq!(trace.outputs, cold.outputs[..1]);
                    assert_eq!(trace.rereads, cold.rereads[..2]);
                }
                assert!(warm.active.iter().all(|rows| rows.is_empty()));
                assert!(warm.reservations.is_empty());
                assert!(warm.seen.is_empty());
            }
            let mut ordinary = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            let trace = discovery_trace_scoped(
                &mut ordinary,
                members,
                Some(Progress {
                    key: &key,
                    target: Some(2),
                }),
                Some(2),
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2);
            assert_eq!(trace.outputs, cold.outputs[2..]);
            assert_eq!(trace.rereads, cold.rereads[4..]);
            assert_eq!(ordinary.seen.len(), 3);
            assert_eq!(ordinary.reservations.len(), 3);
            drop(ordinary);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn pair_frontier_retains_unbound_and_full_hash_collision_errors() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            for collision in [false, true] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let matching = VectorMatching {
                    cardinality,
                    on: true,
                    ..VectorMatching::default()
                };
                let count = if collision { 3 } else { 4 };
                let mut plan = grouped_pair_plan_rows(count, &matching, &control, &account);
                if collision {
                    for (index, value) in crate::test_storage::FULL_LABEL_HASH_COLLISION_VALUES
                        .into_iter()
                        .enumerate()
                    {
                        plan.cache.entries[index].labels = OwnedLabels::copy_from(
                            &[LabelRef { name: b"a", value }],
                            &account,
                            MemoryKind::Workspace,
                        );
                        assert_eq!(
                            label_hash::full(&plan.cache.entries[index].labels, &control, &account)
                                .unwrap(),
                            0x9c71_0b2e_3747_e3bd
                        );
                    }
                }
                plan.certify_replay(&[0, 1, 2], cardinality, &control, &account)
                    .unwrap();
                plan.certify_pair_discovery(cardinality, &control, &account)
                    .unwrap();
                let key = discovery_key();
                let many = usize::from(cardinality == Cardinality::OneToMany);
                let rows: Vec<_> = (0..count).collect();
                let members = if many == 0 {
                    [rows.as_slice(), &[0][..]]
                } else {
                    [&[0][..], rows.as_slice()]
                };
                {
                    let mut cold = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                    cold.enable_pair_discovery(key, &control, &account).unwrap();
                    let trace = discovery_trace_scoped(
                        &mut cold,
                        members,
                        Some(Progress {
                            key: &key,
                            target: Some(0),
                        }),
                        Some(0),
                        &control,
                        &account,
                    );
                    assert_eq!(trace.outputs.len(), 1);
                }
                let mut warm = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
                warm.enable_pair_discovery(key, &control, &account).unwrap();
                assert!(!warm.frontier);
                let progress = Some(Progress {
                    key: &key,
                    target: Some(99),
                });
                let mut reads = Vec::new();
                let failure = loop {
                    match warm.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        progress,
                        &control,
                        &account,
                    ) {
                        Ok(Action::Child {
                            side,
                            ordinal: Some(ordinal),
                        }) => {
                            let row = plan.sides[side]
                                .rows
                                .iter()
                                .position(|row| row.ordinal == *ordinal)
                                .unwrap();
                            reads.push((side, row));
                            warm.receive(
                                discovery_sample(&plan, side, row, &control, &account),
                                &control,
                                &account,
                            )
                            .unwrap();
                        }
                        Ok(Action::Sample(_)) => {}
                        Err(error) => break error,
                        _ => panic!("the ordinary unverified pair error must remain reached"),
                    }
                };
                let expected_rows = if collision { 1..2 } else { 1..4 };
                assert_eq!(
                    reads,
                    expected_rows
                        .flat_map(|row| [(many, row), (1 - many, 0)])
                        .collect::<Vec<_>>()
                );
                assert!(
                    matches!(failure, EvaluationError::Invalid(message) if message ==
                    if collision { "multiple matches for labels: grouping labels must ensure unique matches" }
                    else { "frozen binary label cache key is missing" })
                );
                drop(warm);
                drop(plan);
                account.snapshot().assert_memory_released();
            }
        }
    }

    #[test]
    fn pair_prefix_fresh_suppression_continues_into_an_unvisited_suffix() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching {
            cardinality: Cardinality::ManyToOne,
            on: true,
            ..VectorMatching::default()
        };
        let plan = prefix_pair_plan(&matching, &control, &account);
        let key = discovery_key();
        let progress = Some(Progress {
            key: &key,
            target: Some(0),
        });
        {
            let mut cold = Step::new(&plan, &matching, TokenKind::GTR, false, &account);
            cold.enable_pair_discovery(key, &control, &account).unwrap();
            loop {
                match cold
                    .advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        progress,
                        &control,
                        &account,
                    )
                    .unwrap()
                {
                    Action::Child { side, ordinal } => {
                        let rows: Vec<_> = ordinal.map_or_else(
                            || (0..plan.sides[side].rows.len()).collect(),
                            |ordinal| {
                                vec![
                                    plan.sides[side]
                                        .rows
                                        .iter()
                                        .position(|row| row.ordinal == *ordinal)
                                        .unwrap(),
                                ]
                            },
                        );
                        for row in rows {
                            let mut sample = discovery_sample(&plan, side, row, &control, &account);
                            if ordinal.is_some() && side == 0 && row == 0 {
                                sample.value = OwnedValue::Float(2000.0);
                            }
                            cold.receive(sample, &control, &account).unwrap();
                        }
                    }
                    Action::Sample(sample) => {
                        assert_eq!(sample.ordinal, 0);
                        break;
                    }
                    Action::Done(_) => panic!("the first cold pair must emit"),
                }
            }
        }
        let credits = plan.pair_discovery(&key).unwrap();
        assert_eq!(credits.state(0, &control, &account).unwrap(), Some(true));
        assert_eq!(credits.state(1, &control, &account).unwrap(), None);
        let mut warm = Step::new(&plan, &matching, TokenKind::GTR, false, &account);
        warm.enable_pair_discovery(key, &control, &account).unwrap();
        for (side, row) in [(0, 0), (1, 0)] {
            assert!(
                matches!(warm.advance(NodeId(0), discovery_options(), None, progress, &control, &account).unwrap(),
                Action::Child { side: found, ordinal: Some(ordinal) } if found == side && *ordinal == plan.sides[side].rows[row].ordinal)
            );
            warm.receive(
                discovery_sample(&plan, side, row, &control, &account),
                &control,
                &account,
            )
            .unwrap();
        }
        assert!(
            matches!(warm.advance(NodeId(0), discovery_options(), None, progress, &control, &account).unwrap(),
            Action::Child { side: 0, ordinal: Some(ordinal) } if *ordinal == plan.sides[0].rows[1].ordinal)
        );
        assert_eq!(credits.state(0, &control, &account).unwrap(), Some(false));
        assert_eq!(credits.state(1, &control, &account).unwrap(), None);
        assert!(matches!(
            warm.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            ),
            Err(EvaluationError::Invalid("binary operand replay is empty"))
        ));
        drop(warm);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn grouped_pair_replay_preserves_original_sides_and_completed_clock() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching {
                cardinality,
                on: true,
                ..VectorMatching::default()
            };
            let mut plan = grouped_pair_plan(&matching, &control, &account);
            plan.certify_replay(&[0, 1, 2], cardinality, &control, &account)
                .unwrap();
            let many = usize::from(cardinality == Cardinality::OneToMany);
            let replay = plan.replay.as_ref().expect("unique grouped proof");
            let ReplayRows::Pairs(pairs) = &replay.rows else {
                panic!("pair owner")
            };
            for (row, pair) in pairs.iter().enumerate() {
                assert_eq!(pair[many], row);
                assert_eq!(pair[1 - many], 0);
            }
            let members = if many == 0 {
                [&[0, 1, 2][..], &[0][..]]
            } else {
                [&[0][..], &[0, 1, 2][..]]
            };
            let key = discovery_key();
            let mut incomplete = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            incomplete
                .enable_replay(2, key, &control, &account)
                .unwrap();
            let partial =
                discovery_trace_scoped(&mut incomplete, members, None, Some(0), &control, &account);
            assert_eq!(partial.outputs.len(), 1);
            assert!(replay.completed.borrow().is_none());
            drop(incomplete);
            let mut cold = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            cold.enable_replay(2, key, &control, &account).unwrap();
            let full = discovery_trace(&mut cold, members, &control, &account);
            assert_eq!(full.scans, 2);
            assert_eq!(full.outputs.len(), 3);
            assert_eq!(full.rereads.len(), 6);
            drop(cold);
            let mut warm = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            warm.enable_replay(2, key, &control, &account).unwrap();
            let selected = discovery_trace(&mut warm, members, &control, &account);
            assert_eq!(selected.scans, 0);
            assert_eq!(selected.rereads, [(many, 2), (1 - many, 0)]);
            assert_eq!(selected.outputs, [full.outputs[2].clone()]);
            drop(warm);
            let next = ReplayKey {
                clock: [2000; 9],
                ..key
            };
            let mut empty = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            empty.enable_replay(2, next, &control, &account).unwrap();
            let empty_trace = discovery_trace(&mut empty, [&[], &[]], &control, &account);
            assert_eq!(empty_trace.scans, 2);
            assert!(empty_trace.outputs.is_empty());
            drop(empty);
            let mut absent = Step::new(&plan, &matching, TokenKind::DIV, false, &account);
            absent.enable_replay(2, next, &control, &account).unwrap();
            let absent_trace = discovery_trace(&mut absent, members, &control, &account);
            assert_eq!(absent_trace.scans, 0);
            assert!(absent_trace.outputs.is_empty() && absent_trace.rereads.is_empty());
            drop(absent);
            drop((plan, matching));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn grouped_pair_certification_rejects_ambiguous_one_side_and_missing_outputs() {
        for cardinality in [Cardinality::ManyToOne, Cardinality::OneToMany] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching {
                cardinality,
                on: true,
                ..VectorMatching::default()
            };
            let mut plan = grouped_pair_plan(&matching, &control, &account);
            plan.certify_replay(&[0, 1, 2, 3], cardinality, &control, &account)
                .unwrap();
            assert!(plan.replay.is_none());
            let mut ambiguous = discovery_plan_shape([3, 3], true, &matching, &control, &account);
            ambiguous
                .certify_replay(&[], cardinality, &control, &account)
                .unwrap();
            assert!(ambiguous.replay.is_none());
            drop((plan, ambiguous, matching));
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn set_prefix_preserves_unverified_and_fresh_requested_reads() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan_names(65, true, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let key = discovery_key();
        let members = [&[0, 64][..], &[0, 1, 64][..]];
        for (target, expected_reads, expected_outputs, expected_prefix) in [
            (65, vec![(0, 0), (0, 64), (1, 0)], vec![0, 64, 65], 65),
            (129, vec![(1, 1), (1, 64)], vec![66, 129], 129),
            (64, vec![(0, 64)], vec![64], 129),
            (0, vec![(0, 0)], vec![0], 129),
        ] {
            let cold = !plan.has_completed_set_discovery();
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            let trace = discovery_trace_scoped(
                &mut step,
                members,
                Some(Progress {
                    key: &key,
                    target: Some(target),
                }),
                Some(target),
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2 * usize::from(cold));
            assert_eq!(trace.rereads, expected_reads);
            assert_eq!(
                trace.outputs.iter().map(|row| row.0).collect::<Vec<_>>(),
                expected_outputs
            );
            let selected = trace.outputs.last().unwrap();
            let side = usize::from(target >= 65);
            let row = target - side * 65;
            assert_eq!(selected.0, target);
            assert_eq!(
                selected.1,
                plan.sides[side].rows[row].labels.0[0].value.to_vec()
            );
            assert_eq!(selected.2, ((1000 * side + row + 1) as f64).to_bits());
            assert_eq!(plan.set_prefix(&key), Some(expected_prefix));
        }
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_all_body_seeds_and_done_preserves_progress() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan_names(65, true, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let key = discovery_key();
        let members = [&[0, 64][..], &[0, 1, 64][..]];
        for cold in [true, false] {
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            let trace = discovery_trace_scoped(
                &mut step,
                members,
                Some(Progress {
                    key: &key,
                    target: None,
                }),
                None,
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2 * usize::from(cold));
            assert_eq!(trace.rereads, [(0, 0), (0, 64), (1, 0), (1, 1), (1, 64)]);
            assert_eq!(
                trace.outputs.iter().map(|row| row.0).collect::<Vec<_>>(),
                [0, 64, 65, 66, 129]
            );
            assert_eq!(plan.set_prefix(&key), Some(129));
            step.complete_replay(&control).unwrap();
            assert_eq!(plan.set_prefix(&key), Some(129));
        }
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        let trace = discovery_trace_scoped(
            &mut step,
            members,
            Some(Progress {
                key: &key,
                target: Some(129),
            }),
            Some(129),
            &control,
            &account,
        );
        assert_eq!(trace.rereads, [(1, 64)]);
        assert_eq!(step.seen.len(), 5);
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_inactive_and_wrong_keys_neither_skip_nor_advance() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan_names(4, true, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let key = discovery_key();
        let members = [&[0, 1, 2][..], &[0][..]];
        {
            let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            first.enable_set_discovery(key, &control, &account).unwrap();
            discovery_trace_scoped(
                &mut first,
                members,
                Some(Progress {
                    key: &key,
                    target: Some(0),
                }),
                Some(0),
                &control,
                &account,
            );
        }
        assert_eq!(plan.set_prefix(&key), Some(0));
        let mut keys = vec![None];
        for field in 0..key.clock.len() {
            let mut other = key;
            other.clock[field] += 1;
            keys.push(Some(other));
        }
        keys.extend([
            Some(ReplayKey {
                start_timestamps: true,
                ..key
            }),
            Some(ReplayKey {
                delayed_name_removal: true,
                ..key
            }),
        ]);
        for other in keys {
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            if other.is_some() {
                step.enable_set_discovery(key, &control, &account).unwrap();
            }
            let trace = discovery_trace_scoped(
                &mut step,
                members,
                other.as_ref().map(|key| Progress {
                    key,
                    target: Some(4),
                }),
                Some(4),
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2 * usize::from(other.is_none()));
            assert_eq!(trace.rereads, [(0, 0), (0, 1), (0, 2), (1, 0)]);
            assert_eq!(plan.set_prefix(&key), Some(0));
            if let Some(other) = other {
                assert_eq!(plan.set_prefix(&other), None);
            }
        }
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        let trace = discovery_trace_scoped(
            &mut step,
            members,
            Some(Progress {
                key: &key,
                target: Some(4),
            }),
            Some(4),
            &control,
            &account,
        );
        assert_eq!(trace.rereads, [(0, 1), (0, 2), (1, 0)]);
        assert_eq!(plan.set_prefix(&key), Some(4));
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_discovery_and_incomplete_operand_do_not_grant_credit() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan_names(2, true, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let key = discovery_key();
        let progress = Some(Progress {
            key: &key,
            target: Some(2),
        });
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        for (side, members) in [&[0, 1][..], &[0][..]].into_iter().enumerate() {
            assert!(matches!(
                step.advance(NodeId(0), discovery_options(), None, progress, &control, &account).unwrap(),
                Action::Child { side: found, ordinal: None } if found == side
            ));
            for row in members {
                step.receive(
                    discovery_sample(&plan, side, *row, &control, &account),
                    &control,
                    &account,
                )
                .unwrap();
            }
        }
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            )
            .unwrap(),
            Action::Child {
                side: 0,
                ordinal: Some(_)
            }
        ));
        assert!(plan.has_completed_set_discovery());
        assert_eq!(plan.set_prefix(&key), None);
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            ),
            Err(EvaluationError::Invalid("binary operand replay is empty"))
        ));
        drop(step);
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            )
            .unwrap(),
            Action::Child {
                side: 0,
                ordinal: Some(_)
            }
        ));
        step.receive(
            discovery_sample(&plan, 0, 0, &control, &account),
            &control,
            &account,
        )
        .unwrap();
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            )
            .unwrap(),
            Action::Sample(EvaluatedSample { ordinal: 0, .. })
        ));
        assert_eq!(plan.set_prefix(&key), Some(0));
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            )
            .unwrap(),
            Action::Child {
                side: 0,
                ordinal: Some(_)
            }
        ));
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                progress,
                &control,
                &account
            ),
            Err(EvaluationError::Invalid("binary operand replay is empty"))
        ));
        assert_eq!(plan.set_prefix(&key), Some(0));
        drop(step);
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        let trace =
            discovery_trace_scoped(&mut step, [&[], &[]], progress, Some(2), &control, &account);
        assert_eq!(trace.rereads, [(0, 1), (1, 0)]);
        assert_eq!(plan.set_prefix(&key), Some(2));
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_keeps_selected_failures_and_unverified_missing_suffix() {
        for selected in [true, false] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(4, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            let key = discovery_key();
            let members = [&[0, 1][..], &[0][..]];
            {
                let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                first.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace_scoped(
                    &mut first,
                    members,
                    Some(Progress {
                        key: &key,
                        target: Some(1),
                    }),
                    Some(1),
                    &control,
                    &account,
                );
            }
            assert_eq!(plan.set_prefix(&key), Some(1));
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            let progress = Some(Progress {
                key: &key,
                target: Some(if selected { 1 } else { 99 }),
            });
            assert!(matches!(
                step.advance(NodeId(0), discovery_options(), None, progress, &control, &account).unwrap(),
                Action::Child { side, ordinal: Some(_) } if side == usize::from(!selected)
            ));
            assert!(matches!(
                step.advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    progress,
                    &control,
                    &account
                ),
                Err(EvaluationError::Invalid("binary operand replay is empty"))
            ));
            assert_eq!(plan.set_prefix(&key), Some(1));
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn set_prefix_clear_and_scope_replacement_reset_success() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan_names(4, true, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let base = discovery_key();
        let mut keys = vec![base];
        for field in 0..base.clock.len() {
            let mut key = base;
            key.clock[field] += 1;
            keys.push(key);
        }
        keys.extend([
            ReplayKey {
                start_timestamps: true,
                ..base
            },
            ReplayKey {
                delayed_name_removal: true,
                ..base
            },
            base,
        ]);
        let members = [&[0, 3][..], &[0, 3][..]];
        for key in keys {
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            assert!(!plan.has_completed_set_discovery());
            let ReplayRows::SetDiscovery { prefix } = &plan.replay.as_ref().unwrap().rows else {
                panic!("set owner")
            };
            assert_eq!(prefix.get(), None);
            let trace = discovery_trace_scoped(
                &mut step,
                members,
                Some(Progress {
                    key: &key,
                    target: None,
                }),
                None,
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2);
            assert_eq!(plan.set_prefix(&key), Some(7));
        }
        plan.clear_discovery();
        let ReplayRows::SetDiscovery { prefix } = &plan.replay.as_ref().unwrap().rows else {
            panic!("set owner")
        };
        assert_eq!(prefix.get(), None);
        assert!(!plan.has_completed_set_discovery());
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_reconstructs_seen_and_retains_deferred_duplicates() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let sides = crate::test_storage::FULL_LABEL_HASH_COLLISION_VALUES.map(|value| {
            let labels = OwnedLabels::copy_from(
                &[LabelRef { name: b"a", value }],
                &account,
                MemoryKind::Workspace,
            );
            let hash = label_hash::full(&labels, &control, &account).unwrap();
            assert_eq!(hash, 0x9c71_0b2e_3747_e3bd);
            let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
            rows.push(Descriptor {
                labels,
                hash,
                ordinal: 0,
                drop_name: false,
                first_order: 0,
                sources: 0..0,
                kinds: 1,
                heads: Default::default(),
            });
            rows
        });
        let [left, right] = sides;
        let mut plan = Plan::new(left, right, &matching, &control, &account).unwrap();
        assert_eq!(plan.signature_count, 2);
        plan.certify_set_discovery(&control, &account).unwrap();
        assert!(plan.has_set_discovery());
        let key = discovery_key();
        {
            let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            first.enable_set_discovery(key, &control, &account).unwrap();
            let trace = discovery_trace_scoped(
                &mut first,
                [&[0], &[0]],
                Some(Progress {
                    key: &key,
                    target: Some(1),
                }),
                Some(1),
                &control,
                &account,
            );
            assert_eq!(trace.scans, 2);
            assert_eq!(trace.rereads, [(0, 0), (1, 0)]);
            assert_eq!(trace.outputs.len(), 2);
            assert_ne!(trace.outputs[0].1, trace.outputs[1].1);
            assert!(first.duplicate);
        }
        assert_eq!(plan.set_prefix(&key), Some(1));
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(key, &control, &account).unwrap();
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                Some(Progress {
                    key: &key,
                    target: Some(99)
                }),
                &control,
                &account,
            ),
            Err(EvaluationError::Invalid(
                "vector cannot contain metrics with the same labelset"
            ))
        ));
        assert!(matches!(step.phase, Phase::Finished));
        assert_eq!(step.seen.len(), 1);
        assert!(step.duplicate);
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_prefix_preserves_each_operator_and_suppressed_membership() {
        for (operator, expected) in [
            (TokenKind::LOR, vec![0, 1, 64, 67]),
            (TokenKind::LAND, vec![0, 64]),
            (TokenKind::LUNLESS, vec![1]),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan(65, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            let key = discovery_key();
            let members = [&[0, 1, 64][..], &[0, 2, 64][..]];
            let complete = {
                let mut step = Step::new(&plan, &matching, operator, false, &account);
                step.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace_scoped(
                    &mut step,
                    members,
                    Some(Progress {
                        key: &key,
                        target: None,
                    }),
                    None,
                    &control,
                    &account,
                )
            };
            assert_eq!(
                complete.outputs.iter().map(|row| row.0).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(plan.set_prefix(&key), expected.last().copied());
            for selected in complete.outputs.iter().rev() {
                let mut step = Step::new(&plan, &matching, operator, false, &account);
                step.enable_set_discovery(key, &control, &account).unwrap();
                let trace = discovery_trace_scoped(
                    &mut step,
                    [&[], &[]],
                    Some(Progress {
                        key: &key,
                        target: Some(selected.0),
                    }),
                    Some(selected.0),
                    &control,
                    &account,
                );
                assert_eq!(trace.scans, 0);
                assert_eq!(trace.rereads.len(), 1);
                assert_eq!(trace.outputs.as_slice(), std::slice::from_ref(selected));
            }
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    fn pending_prefix_value<'a>(
        plan: &'a Plan,
        matching: &'a VectorMatching,
        key: &ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Step<'a> {
        let mut step = Step::new(plan, matching, TokenKind::LOR, false, account);
        step.enable_set_discovery(*key, control, account).unwrap();
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                Some(Progress {
                    key,
                    target: Some(0)
                }),
                control,
                account,
            )
            .unwrap(),
            Action::Child {
                side: 0,
                ordinal: Some(_)
            }
        ));
        step.receive(
            discovery_sample(plan, 0, 0, control, account),
            control,
            account,
        )
        .unwrap();
        step
    }

    #[test]
    fn set_prefix_cancellation_precedes_terminal_credit_and_reconstruction() {
        let key = discovery_key();
        let members = [&[0, 1][..], &[0][..]];
        let emission_work = {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(4, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            {
                let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                first.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace(&mut first, members, &control, &account);
            }
            let mut step = pending_prefix_value(&plan, &matching, &key, &control, &account);
            let before = account.snapshot().work;
            let action = step
                .advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    Some(Progress {
                        key: &key,
                        target: Some(0),
                    }),
                    &control,
                    &account,
                )
                .unwrap();
            assert!(matches!(
                action,
                Action::Sample(EvaluatedSample { ordinal: 0, .. })
            ));
            let work = account.snapshot().work - before;
            assert_eq!(plan.set_prefix(&key), Some(0));
            drop(action);
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
            work
        };
        for budget in [1, emission_work] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(4, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            {
                let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                first.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace(&mut first, members, &control, &account);
            }
            let mut step = pending_prefix_value(&plan, &matching, &key, &control, &account);
            let result =
                crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                    step.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        Some(Progress {
                            key: &key,
                            target: Some(0),
                        }),
                        &control,
                        &account,
                    )
                });
            assert!(
                matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ),
                "budget {budget}"
            );
            assert_eq!(plan.set_prefix(&key), None);
            drop(result);
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
        for budget in [1, 7, 16, 32] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(65, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            {
                let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                first.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace_scoped(
                    &mut first,
                    members,
                    Some(Progress {
                        key: &key,
                        target: None,
                    }),
                    None,
                    &control,
                    &account,
                );
            }
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            let result =
                crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                    step.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        Some(Progress {
                            key: &key,
                            target: Some(999),
                        }),
                        &control,
                        &account,
                    )
                });
            assert!(
                matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ),
                "budget {budget}"
            );
            assert_eq!(plan.set_prefix(&key), Some(65));
            drop(result);
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    fn pending_prefix_skip<'a>(
        plan: &'a Plan,
        matching: &'a VectorMatching,
        key: &ReplayKey,
        control: &Control,
        account: &QueryAccounting,
    ) -> Step<'a> {
        let mut step = Step::new(plan, matching, TokenKind::LOR, false, account);
        step.enable_set_discovery(*key, control, account).unwrap();
        assert!(matches!(step.phase, Phase::Begin));
        step.prepare_active(0, control, account).unwrap();
        step.phase = Phase::Pairs;
        let progress = Some(Progress {
            key,
            target: Some(999),
        });
        for row in [0, 1] {
            assert!(step.choose(control, account).unwrap());
            assert!(
                matches!(step.choice, Some(Choice::Set(Operand { side: 0, row: selected, .. })) if selected == row)
            );
            step.phase = Phase::ReadLeft;
            assert!(step.skip_set_prefix(progress, control, account).unwrap());
        }
        assert!(step.choose(control, account).unwrap());
        assert!(matches!(
            step.choice,
            Some(Choice::Set(Operand {
                side: 1,
                row: 0,
                ..
            }))
        ));
        step.phase = Phase::ReadLeft;
        assert_eq!(step.seen.len(), 2);
        step
    }

    #[test]
    fn set_prefix_skipped_hashes_poll_after_seen_reconstruction() {
        let key = discovery_key();
        let members = [&[0, 1][..], &[0][..]];
        let mut terminal_work = 0;
        for boundary in 0..3 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(65, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            {
                let mut first = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                first.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace_scoped(
                    &mut first,
                    members,
                    Some(Progress {
                        key: &key,
                        target: None,
                    }),
                    None,
                    &control,
                    &account,
                );
            }
            assert_eq!(plan.set_prefix(&key), Some(65));
            let mut step = pending_prefix_skip(&plan, &matching, &key, &control, &account);
            let before = account.snapshot().work;
            let budget = if boundary == 1 { 1 } else { terminal_work };
            let mut operation = || {
                step.skip_set_prefix(
                    Some(Progress {
                        key: &key,
                        target: Some(999),
                    }),
                    &control,
                    &account,
                )
            };
            let result = if boundary == 0 {
                operation()
            } else {
                crate::test_storage::interrupt_after_work(&account, &control, budget, operation)
            };
            if boundary == 0 {
                assert!(matches!(result, Ok(true)));
                assert!(matches!(step.phase, Phase::Pairs));
                assert_eq!(step.seen.len(), 3);
                terminal_work = account.snapshot().work - before;
                assert!(terminal_work > 1);
            } else {
                assert!(matches!(
                    result,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ));
                assert!(matches!(step.phase, Phase::ReadLeft));
                if boundary == 2 {
                    assert_eq!(step.seen.len(), 3);
                    assert_eq!(account.snapshot().work - before, terminal_work);
                }
            }
            assert!(!step.duplicate);
            assert_eq!(plan.set_prefix(&key), Some(65));
            println!(
                "{{\"skipped_hash_boundary\":{boundary},\"work\":{},\"terminal_work\":{terminal_work},\"seen\":{}}}",
                account.snapshot().work - before,
                step.seen.len()
            );
            drop(result);
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn completed_set_discovery_keeps_suppressed_inputs_and_original_rereads() {
        for duplicate_signatures in [false, true] {
            for operator in [TokenKind::LOR, TokenKind::LAND, TokenKind::LUNLESS] {
                let engine = EngineAccounting::default();
                let account = engine.query();
                let control = Control::default();
                let matching = VectorMatching {
                    on: duplicate_signatures,
                    labels: duplicate_signatures
                        .then(|| AccountedVec::new(&account, MemoryKind::Workspace)),
                    ..VectorMatching::default()
                };
                let mut plan = discovery_plan(65, &matching, &control, &account);
                plan.certify_set_discovery(&control, &account).unwrap();
                assert!(plan.has_set_discovery());
                assert!(
                    plan.sides
                        .iter()
                        .all(|side| side.ordinal_positions.is_some())
                );
                let members = [&[0, 64][..], &[0, 1, 64][..]];
                let cold = {
                    let mut step = Step::new(&plan, &matching, operator, false, &account);
                    step.enable_set_discovery(discovery_key(), &control, &account)
                        .unwrap();
                    discovery_trace(&mut step, members, &control, &account)
                };
                assert_eq!(cold.scans, 2);
                let snapshot_guard = plan.replay.as_ref().unwrap().completed.borrow();
                let snapshot = snapshot_guard.as_ref().unwrap();
                assert_eq!(snapshot.members.len(), 3);
                assert_eq!(snapshot.members[0], 1);
                assert_eq!(snapshot.members[1], 7);
                assert_eq!(snapshot.members[2], 2);
                drop(snapshot_guard);
                let warm = {
                    let mut step = Step::new(&plan, &matching, operator, false, &account);
                    step.enable_set_discovery(discovery_key(), &control, &account)
                        .unwrap();
                    assert!(step.pending.is_none());
                    discovery_trace(&mut step, [&[], &[]], &control, &account)
                };
                assert_eq!(warm.scans, 0);
                assert_eq!(warm.outputs, cold.outputs);
                assert_eq!(warm.rereads, cold.rereads);
                let expected_rows: &[usize] = match (operator, duplicate_signatures) {
                    (TokenKind::LOR, false) => &[0, 64, 66],
                    (TokenKind::LOR | TokenKind::LAND, _) => &[0, 64],
                    (TokenKind::LUNLESS, _) => &[],
                    _ => unreachable!(),
                };
                assert_eq!(
                    warm.outputs.iter().map(|row| row.0).collect::<Vec<_>>(),
                    expected_rows
                );
                drop((plan, matching));
                account.snapshot().assert_memory_released();
            }
        }
        println!(
            "set discovery layouts: ReplayRows={} Replay={} CompletedStep={} Plan={} Step={}",
            size_of::<ReplayRows>(),
            size_of::<Replay>(),
            size_of::<CompletedStep>(),
            size_of::<Plan>(),
            size_of::<Step<'_>>()
        );
        assert_eq!(
            size_of::<ReplayRows>(),
            size_of::<AccountedVec<[usize; 2]>>()
        );
    }

    #[test]
    fn set_discovery_replaces_each_exact_scope_without_overlapping_snapshots() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan(65, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let base = discovery_key();
        let mut keys = vec![base];
        for field in 0..base.clock.len() {
            let mut key = base;
            key.clock[field] += 1;
            keys.push(key);
        }
        keys.push(ReplayKey {
            start_timestamps: true,
            ..base
        });
        keys.push(ReplayKey {
            delayed_name_removal: true,
            ..base
        });
        for (index, key) in keys.into_iter().enumerate() {
            let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            assert!(step.pending.is_some());
            assert!(!plan.has_completed_set_discovery());
            if index != 0 {
                assert_eq!(
                    account.snapshot().memory[MemoryKind::Workspace as usize].current,
                    before
                );
            }
            let members = if index % 2 == 0 {
                [&[0][..], &[][..]]
            } else {
                [&[][..], &[64][..]]
            };
            let trace = discovery_trace(&mut step, members, &control, &account);
            assert_eq!(trace.scans, 2);
            assert_eq!(trace.outputs.len(), 1);
            assert_eq!(trace.outputs[0].0, if index % 2 == 0 { 0 } else { 129 });
            drop(step);
        }
        plan.clear_discovery();
        assert!(plan.has_set_discovery());
        assert!(!plan.has_completed_set_discovery());
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn set_discovery_publishes_after_both_streams_before_selected_rereads() {
        for abandoned_phase in 0..3 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan(65, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(discovery_key(), &control, &account)
                .unwrap();
            assert!(matches!(
                step.advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    None,
                    &control,
                    &account
                )
                .unwrap(),
                Action::Child {
                    side: 0,
                    ordinal: None
                }
            ));
            for row in [0, 64] {
                step.receive(
                    discovery_sample(&plan, 0, row, &control, &account),
                    &control,
                    &account,
                )
                .unwrap();
            }
            if abandoned_phase > 0 {
                assert!(matches!(
                    step.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        None,
                        &control,
                        &account
                    )
                    .unwrap(),
                    Action::Child {
                        side: 1,
                        ordinal: None
                    }
                ));
                for row in [0, 1, 64] {
                    step.receive(
                        discovery_sample(&plan, 1, row, &control, &account),
                        &control,
                        &account,
                    )
                    .unwrap();
                }
            }
            assert!(!plan.has_completed_set_discovery());
            if abandoned_phase == 2 {
                assert!(matches!(
                    step.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        None,
                        &control,
                        &account
                    )
                    .unwrap(),
                    Action::Child {
                        side: 0,
                        ordinal: Some(_)
                    }
                ));
                assert!(plan.has_completed_set_discovery());
                assert!(step.pending.is_none());
                assert!(matches!(
                    step.advance(
                        NodeId(0),
                        discovery_options(),
                        None,
                        None,
                        &control,
                        &account
                    ),
                    Err(EvaluationError::Invalid("binary operand replay is empty"))
                ));
            }
            drop(step);
            assert_eq!(plan.has_completed_set_discovery(), abandoned_phase == 2);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn set_discovery_discards_duplicate_descending_and_native_capture() {
        for case in 0..3 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan(65, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(discovery_key(), &control, &account)
                .unwrap();
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                None,
                &control,
                &account,
            )
            .unwrap();
            let rows = if case == 1 { [64, 0] } else { [0, 0] };
            for row in rows {
                let mut sample = discovery_sample(&plan, 0, row, &control, &account);
                if case == 2 {
                    let histogram = crate::histogram::FloatHistogram::default();
                    sample.value = OwnedValue::copy_from(
                        crate::value::SampleValueRef::FloatHistogram(histogram.as_ref()),
                        &account,
                        MemoryKind::Workspace,
                    );
                }
                step.receive(sample, &control, &account).unwrap();
            }
            assert!(step.pending.is_none());
            assert_eq!(step.active[0].len(), 2);
            assert_eq!(step.active[0][0].row, rows[0]);
            assert_eq!(step.active[0][1].row, rows[1]);
            assert!(matches!(
                step.advance(
                    NodeId(0),
                    discovery_options(),
                    None,
                    None,
                    &control,
                    &account
                )
                .unwrap(),
                Action::Child {
                    side: 1,
                    ordinal: None
                }
            ));
            assert!(!plan.has_completed_set_discovery());
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn set_discovery_certification_rejects_ambiguous_owned_rows() {
        for case in 0..5 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan(4, &matching, &control, &account);
            match case {
                0 => plan.sides[0].rows[1].hash = plan.sides[0].rows[0].hash,
                1 => {
                    let mut counts = crate::evaluator_roster::PointCounts::default();
                    counts.push(1);
                    counts.push(1);
                    plan.sides[0].rows[0].heads.note(counts, 1000);
                }
                2 => plan.sides[0].rows[0].sources = 0..2,
                3 => plan.sides[0].rows[0].kinds = 2,
                4 => plan.sides[0].rows[1].ordinal = plan.sides[0].rows[0].ordinal,
                _ => unreachable!(),
            }
            plan.certify_set_discovery(&control, &account).unwrap();
            assert!(!plan.has_set_discovery(), "case {case}");
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn set_discovery_sparse_reconstruction_avoids_inactive_rows() {
        let mut work = Vec::new();
        for count in [128, 512] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan_names(count, true, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            let key = discovery_key();
            let last = count - 1;
            let members = [&[0, last][..], &[last][..]];
            let cold = {
                let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                step.enable_set_discovery(key, &control, &account).unwrap();
                discovery_trace(&mut step, members, &control, &account)
            };
            assert_eq!(cold.scans, 2);
            assert_eq!(cold.rereads, [(0, 0), (0, last), (1, last)]);
            assert_eq!(
                cold.outputs,
                [
                    (0, b"00000000".to_vec(), 1.0f64.to_bits()),
                    (
                        last,
                        format!("{last:08}").into_bytes(),
                        (count as f64).to_bits()
                    ),
                    (
                        count + last,
                        format!("r{last:08}").into_bytes(),
                        (1000.0 + count as f64).to_bits()
                    )
                ]
            );
            let before = account.snapshot().work;
            let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            step.enable_set_discovery(key, &control, &account).unwrap();
            let reconstruction_work = account.snapshot().work - before;
            let warm = discovery_trace(&mut step, [&[], &[]], &control, &account);
            assert_eq!(warm.scans, 0);
            assert_eq!(warm.rereads, cold.rereads);
            assert_eq!(warm.outputs, cold.outputs);
            assert!(plan.has_completed_set_discovery());
            println!(
                "{{\"sparse_rows_per_side\":{count},\"active\":3,\"reconstruction_work\":{reconstruction_work}}}"
            );
            work.push((2 * count, reconstruction_work));
            drop(step);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
        assert!(
            work.iter().all(|(rows, work)| *work <= (*rows / 16) as u64),
            "sparse reconstruction must avoid scanning inactive rows: {work:?}"
        );
    }

    #[test]
    fn set_discovery_bitmap_preserves_side_tail_and_physical_order() {
        for counts in [
            [0, 0],
            [0, 1],
            [1, 0],
            [0, 65],
            [65, 0],
            [1, 1],
            [63, 1],
            [64, 1],
            [65, 65],
            [129, 129],
        ] {
            for operator in [TokenKind::LOR, TokenKind::LAND, TokenKind::LUNLESS] {
                for density in 0..3 {
                    let engine = EngineAccounting::default();
                    let account = engine.query();
                    let control = Control::default();
                    let matching = VectorMatching::default();
                    let mut plan =
                        discovery_plan_shape(counts, false, &matching, &control, &account);
                    plan.certify_set_discovery(&control, &account).unwrap();
                    let members = counts.map(|count| match density {
                        0 => Vec::new(),
                        1 => [0, 1, 63, 64, 65, 128]
                            .into_iter()
                            .filter(|row| *row < count)
                            .collect(),
                        _ => (0..count).collect(),
                    });
                    let cold = {
                        let mut step = Step::new(&plan, &matching, operator, false, &account);
                        step.enable_set_discovery(discovery_key(), &control, &account)
                            .unwrap();
                        discovery_trace(&mut step, [&members[0], &members[1]], &control, &account)
                    };
                    assert_eq!(cold.scans, 2);
                    let rows = counts.iter().sum::<usize>();
                    if rows < 2 {
                        assert!(!plan.has_set_discovery());
                        assert!(!plan.has_completed_set_discovery());
                        drop(plan);
                        account.snapshot().assert_memory_released();
                        continue;
                    }
                    if rows % 64 != 0 {
                        let mut completed = plan.replay.as_ref().unwrap().completed.borrow_mut();
                        let last = completed.as_mut().unwrap().members.last_mut().unwrap();
                        *last |= u64::MAX << (rows % 64);
                    }
                    let before = account.snapshot().work;
                    let mut step = Step::new(&plan, &matching, operator, false, &account);
                    step.enable_set_discovery(discovery_key(), &control, &account)
                        .unwrap();
                    assert_eq!(
                        account.snapshot().work - before,
                        (rows.div_ceil(64) + members[0].len() + members[1].len()) as u64
                    );
                    for (side, members) in members.iter().enumerate() {
                        assert_eq!(
                            step.active[side]
                                .iter()
                                .map(|active| active.row)
                                .collect::<Vec<_>>(),
                            *members
                        );
                    }
                    let warm = discovery_trace(&mut step, [&[], &[]], &control, &account);
                    assert_eq!(warm.scans, 0);
                    assert_eq!(warm.rereads, cold.rereads);
                    assert_eq!(warm.outputs, cold.outputs);
                    assert!(plan.has_completed_set_discovery());
                    drop(step);
                    drop(plan);
                    account.snapshot().assert_memory_released();
                }
            }
        }
    }

    #[test]
    fn set_discovery_cancellation_stops_reconstruction_and_publication() {
        let mut terminal_work = 0;
        for boundary in 0..=8 {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let matching = VectorMatching::default();
            let mut plan = discovery_plan(129, &matching, &control, &account);
            plan.certify_set_discovery(&control, &account).unwrap();
            {
                let mut cold = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
                cold.enable_set_discovery(discovery_key(), &control, &account)
                    .unwrap();
                discovery_trace(&mut cold, [&[0, 128], &[64]], &control, &account);
            }
            let mut warm = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
            let before = account.snapshot().work;
            let result = if boundary == 0 {
                warm.enable_set_discovery(discovery_key(), &control, &account)
            } else {
                let budget = if boundary == 8 {
                    terminal_work
                } else {
                    boundary
                };
                crate::test_storage::interrupt_after_work(&account, &control, budget, || {
                    warm.enable_set_discovery(discovery_key(), &control, &account)
                })
            };
            if boundary == 0 {
                assert!(result.is_ok());
                assert!(matches!(warm.phase, Phase::Begin));
                terminal_work = account.snapshot().work - before;
                assert_eq!(terminal_work, 8);
            } else {
                assert!(
                    matches!(result, Err(Interrupted::Cancelled)),
                    "boundary {boundary}"
                );
                assert!(matches!(warm.phase, Phase::StartLeft));
                assert_eq!(account.snapshot().work - before, boundary);
                if boundary == 8 {
                    assert_eq!(warm.active[0].len(), 2);
                    assert_eq!(warm.active[1].len(), 1);
                }
            }
            assert!(plan.has_completed_set_discovery());
            assert_eq!(plan.set_prefix(&discovery_key()), None);
            drop(warm);
            drop(plan);
            account.snapshot().assert_memory_released();
        }
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let matching = VectorMatching::default();
        let mut plan = discovery_plan(4, &matching, &control, &account);
        plan.certify_set_discovery(&control, &account).unwrap();
        let mut step = Step::new(&plan, &matching, TokenKind::LOR, false, &account);
        step.enable_set_discovery(discovery_key(), &control, &account)
            .unwrap();
        step.advance(
            NodeId(0),
            discovery_options(),
            None,
            None,
            &control,
            &account,
        )
        .unwrap();
        step.receive(
            discovery_sample(&plan, 0, 0, &control, &account),
            &control,
            &account,
        )
        .unwrap();
        step.advance(
            NodeId(0),
            discovery_options(),
            None,
            None,
            &control,
            &account,
        )
        .unwrap();
        control.cancellation.cancel();
        assert!(matches!(
            step.advance(
                NodeId(0),
                discovery_options(),
                None,
                None,
                &control,
                &account
            ),
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        assert!(!plan.has_completed_set_discovery());
        drop(step);
        drop(plan);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn ordinal_lookup_keeps_original_first_matching_rows_and_labels() {
        for (source, expected, unordered) in [
            (
                [(8, b"a"), (3, b"b"), (8, b"c"), (8, b"a"), (1, b"d")],
                [0, 1, 2, 0, 4],
                true,
            ),
            (
                [(1, b"d"), (3, b"b"), (8, b"a"), (8, b"c"), (8, b"a")],
                [0, 1, 2, 3, 2],
                false,
            ),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let control = Control::default();
            let mut rows = AccountedVec::new(&account, MemoryKind::Workspace);
            for (position, (ordinal, value)) in source.into_iter().enumerate() {
                rows.push(Descriptor {
                    labels: labels(value, &account),
                    ordinal,
                    hash: 0,
                    drop_name: false,
                    first_order: position,
                    sources: 0..0,
                    kinds: 1,
                    heads: Default::default(),
                });
            }
            let plan = Plan::new(
                rows,
                AccountedVec::new(&account, MemoryKind::Workspace),
                &VectorMatching::default(),
                &control,
                &account,
            )
            .unwrap();
            assert_eq!(plan.sides[0].ordinal_positions.is_some(), unordered);
            for (position, (ordinal, value)) in source.into_iter().enumerate() {
                let sample = EvaluatedSample {
                    ordinal,
                    labels: labels(value, &account).into(),
                    value: OwnedValue::Float(1.0),
                    float_samples: None,
                    source_time_ms: 0,
                    source_start_time_ms: 0,
                    drop_name: false,
                };
                assert_eq!(
                    plan.position(0, &sample, &control, &account).unwrap(),
                    expected[position]
                );
                assert_eq!(plan.sides[0].rows[position].ordinal, ordinal);
            }
            drop(plan);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn frozen_cache_digest_collisions_still_require_complete_keys_after_rebuild() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let mut target = AccountedVec::new(&account, MemoryKind::Workspace);
        binary_labels::append(
            &mut target,
            &labels(b"target", &account),
            None,
            &control,
            &account,
        )
        .unwrap();
        let hash = label_hash::bytes(&target, &control, &account).unwrap();
        let mut cache = Cache::new(&account);
        for position in 0..128 {
            let value = if position == 63 {
                b"target".to_vec()
            } else {
                format!("other-{position}").into_bytes()
            };
            let labels = labels(&value, &account);
            let mut bytes = AccountedVec::new(&account, MemoryKind::Workspace);
            binary_labels::append(&mut bytes, &labels, None, &control, &account).unwrap();
            // A forced equal digest exercises the owner's complete-key predicate.
            cache.entries.push(CacheEntry {
                key: Key { bytes, hash },
                labels,
            });
            cache
                .index
                .insert_new(
                    hash,
                    position,
                    |prior| Ok(cache.entries[prior].key.hash),
                    &control,
                    &account,
                )
                .unwrap();
        }
        let before = account.snapshot();
        let (position, result) = frozen_labels(&cache, &target, &control, &account).unwrap();
        let lookup_work = account.snapshot().work - before.work;
        assert_eq!(position, 63);
        assert!(
            matches!(result, SampleLabels::Borrowed(value) if std::ptr::eq(value, &cache.entries[position].labels))
        );
        assert_eq!(&*result.0[0].value, b"target");
        assert_eq!(account.snapshot().memory, before.memory);
        drop(result);
        assert!(lookup_work > 0);
        let cancelled = Control::default();
        let result =
            crate::test_storage::interrupt_after_work(&account, &cancelled, lookup_work, || {
                frozen_labels(&cache, &target, &cancelled, &account)
            });
        assert!(matches!(
            result,
            Err(EvaluationError::Interrupted(Interrupted::Cancelled))
        ));
        drop(result);
        let empty_key = AccountedVec::new(&account, MemoryKind::Workspace);
        let empty_hash = label_hash::bytes(&empty_key, &control, &account).unwrap();
        let empty_position = cache.entries.len();
        cache.entries.push(CacheEntry {
            key: Key {
                bytes: empty_key,
                hash: empty_hash,
            },
            labels: OwnedLabels::new(&account, MemoryKind::Workspace),
        });
        cache
            .index
            .insert_new(
                empty_hash,
                empty_position,
                |prior| Ok(cache.entries[prior].key.hash),
                &control,
                &account,
            )
            .unwrap();
        let before = account.snapshot().memory;
        let (position, result) = frozen_labels(&cache, &[], &control, &account).unwrap();
        assert_eq!(position, empty_position);
        assert!(
            matches!(result, SampleLabels::Borrowed(value) if std::ptr::eq(value, &cache.entries[position].labels))
        );
        assert!(result.0.is_empty());
        assert_eq!(account.snapshot().memory, before);
        drop(result);
        assert_eq!(size_of::<Active>(), size_of::<usize>() * 2);
        eprintln!(
            "index layout: Active={} Side={} Cache={} PositionIndex={}",
            size_of::<Active>(),
            size_of::<Side>(),
            size_of::<Cache>(),
            size_of::<PositionIndex>()
        );
        drop((cache, target));
        account.snapshot().assert_memory_released();
    }
}
