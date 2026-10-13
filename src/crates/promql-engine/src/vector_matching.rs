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
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

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
}
pub(crate) struct Plan {
    pub(crate) sides: [Side; 2],
    pub(crate) cache: Cache,
    signature_count: usize,
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
        })
    }
    fn position(
        &self,
        side: usize,
        sample: &EvaluatedSample,
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
    Sample(EvaluatedSample),
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
    phase: Phase,
    position: usize,
    retry: bool,
    choice: Option<Choice>,
    values: [Option<crate::statistics::ValueParts>; 2],
    last_diagnostic: Option<Diagnostic>,
}
impl<'a> Step<'a> {
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
            phase: Phase::StartLeft,
            position: 0,
            retry: false,
            choice: None,
            values: [None, None],
            last_diagnostic: None,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        match self.phase {
            Phase::Left | Phase::Right => {
                let side = usize::from(matches!(self.phase, Phase::Right));
                let row = self.plan.position(side, &sample, control, account)?;
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
    fn labels(
        &self,
        operand: Operand,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<OperandLabels<'a>, Interrupted> {
        let labels = &self.plan.sides[operand.side].rows[operand.row].labels;
        Ok(if operand.fill.is_some() {
            OperandLabels::Owned(binary_labels::matched(
                labels,
                self.matching,
                control,
                account,
            )?)
        } else {
            OperandLabels::Borrowed(labels)
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
        if let Some(position) = cache.index.find(
            hash,
            |position| {
                Ok(cache.entries[position].key.hash == hash
                    && binary_labels::equal(
                        &cache.entries[position].key.bytes,
                        &key,
                        control,
                        account,
                    )?)
            },
            control,
            account,
        )? {
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
    pub(crate) fn advance(
        &mut self,
        node: NodeId,
        options: EvaluationOptions,
        preparation: Option<Preparation<'a>>,
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
                Phase::Right => self.phase = Phase::Begin,
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
                    self.phase = if self.choose(control, account)? {
                        Phase::ReadLeft
                    } else {
                        Phase::Finished
                    };
                }
                Phase::ReadLeft | Phase::ReadRight => {
                    let index = usize::from(matches!(self.phase, Phase::ReadRight));
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
                            (
                                operand.row
                                    + if operand.side == 0 {
                                        0
                                    } else {
                                        self.plan.sides[0].rows.len()
                                    },
                                descriptor.labels.copy_checked(
                                    account,
                                    MemoryKind::Workspace,
                                    control,
                                )?,
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
                                )?
                            } else {
                                frozen_labels(&self.plan.cache, &key, control, account)?
                            };
                            let hash = label_hash::full(&labels, control, account)?;
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
                                        |position| {
                                            Ok(self.reservations[position] == (signature, hash))
                                        },
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
                            if self.return_bool {
                                value = OwnedValue::Float(f64::from(pair.keep));
                            } else if !pair.keep {
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
enum OperandLabels<'a> {
    Borrowed(&'a OwnedLabels),
    Owned(OwnedLabels),
}
impl OperandLabels<'_> {
    fn as_ref(&self) -> &OwnedLabels {
        match self {
            Self::Borrowed(value) => value,
            Self::Owned(value) => value,
        }
    }
}
fn reservation_hash(signature: usize, hash: u64) -> u64 {
    label_hash::avalanche(signature as u64) ^ hash.rotate_left(17)
}
fn frozen_labels(
    cache: &Cache,
    key: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(usize, OwnedLabels), EvaluationError> {
    let hash = label_hash::bytes(key, control, account)?;
    if let Some(position) = cache.index.find(
        hash,
        |position| {
            Ok(cache.entries[position].key.hash == hash
                && binary_labels::equal(&cache.entries[position].key.bytes, key, control, account)?)
        },
        control,
        account,
    )? {
        return Ok((
            position,
            cache.entries[position]
                .labels
                .copy_checked(account, MemoryKind::Workspace, control)?,
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
                    labels: labels(value, &account),
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
        let (position, result) = frozen_labels(&cache, &target, &control, &account).unwrap();
        assert_eq!(position, 63);
        assert_eq!(&*result.0[0].value, b"target");
        assert_eq!(size_of::<Active>(), size_of::<usize>() * 2);
        eprintln!(
            "index layout: Active={} Side={} Cache={} PositionIndex={}",
            size_of::<Active>(),
            size_of::<Side>(),
            size_of::<Cache>(),
            size_of::<PositionIndex>()
        );
        drop((cache, target, result));
        account.snapshot().assert_memory_released();
    }
}
