// SPDX-License-Identifier: GPL-3.0-or-later
// Bounded pair replay follows pinned promql/engine.go:3172-3462; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    binary_labels,
    binary_value::{self, Diagnostic},
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Cardinality, NodeId, VectorMatching},
        token::TokenKind,
    },
    label_hash,
    result::{OwnedLabels, OwnedValue, append_bytes_checked},
    storage::{Control, Interrupted},
};
use std::cell::RefCell;

pub(crate) struct Side {
    pub(crate) rows: AccountedVec<Descriptor>,
    signatures: AccountedVec<usize>,
}
pub(crate) struct CacheEntry {
    key: AccountedVec<u8>,
    labels: OwnedLabels,
}
pub(crate) struct Plan {
    pub(crate) sides: [Side; 2],
    pub(crate) cache: AccountedVec<CacheEntry>,
}
#[derive(Clone, Copy)]
pub(crate) struct Preparation<'a> {
    pub(crate) node: NodeId,
    pub(crate) cache: &'a RefCell<AccountedVec<CacheEntry>>,
}
impl Plan {
    pub(crate) fn new(
        left: AccountedVec<Descriptor>,
        right: AccountedVec<Descriptor>,
        matching: &VectorMatching,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Self, EvaluationError> {
        let mut keys = AccountedVec::<AccountedVec<u8>>::new(account, MemoryKind::Workspace);
        let mut sides = [
            Side {
                rows: left,
                signatures: AccountedVec::new(account, MemoryKind::Workspace),
            },
            Side {
                rows: right,
                signatures: AccountedVec::new(account, MemoryKind::Workspace),
            },
        ];
        for side in &mut sides {
            for row in &*side.rows {
                let mut key = AccountedVec::new(account, MemoryKind::Workspace);
                binary_labels::append(&mut key, &row.labels, Some(matching), control, account)?;
                let mut found = None;
                for (index, prior) in keys.iter().enumerate() {
                    control.check()?;
                    account.work(1);
                    if binary_labels::equal(&key, prior, control, account)? {
                        found = Some(index);
                        break;
                    }
                }
                let index = if let Some(index) = found {
                    index
                } else {
                    let index = keys.len();
                    keys.push(key);
                    index
                };
                side.signatures.push(index);
            }
        }
        Ok(Self {
            sides,
            cache: AccountedVec::new(account, MemoryKind::Workspace),
        })
    }
    fn position(
        &self,
        side: usize,
        sample: &EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, EvaluationError> {
        for (index, row) in self.sides[side].rows.iter().enumerate() {
            control.check()?;
            account.work(1);
            if row.ordinal == sample.ordinal
                && binary_labels::same(&row.labels, &sample.labels, control, account)?
            {
                return Ok(index);
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
    reservations: AccountedVec<(usize, u64)>,
    seen: AccountedVec<u64>,
    duplicate: bool,
    phase: Phase,
    position: usize,
    retry: bool,
    choice: Option<Choice>,
    values: [Option<OwnedValue>; 2],
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
            reservations: AccountedVec::new(account, MemoryKind::Workspace),
            seen: AccountedVec::new(account, MemoryKind::Workspace),
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
                });
            }
            Phase::WaitLeft | Phase::WaitRight => {
                let index = usize::from(matches!(self.phase, Phase::WaitRight));
                let operand = self.operand(index).expect("pending pair operand");
                if self.values[index].is_none() && sample.value.kind_bit() == operand.kind {
                    self.values[index] = Some(sample.value);
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
        cache: &mut AccountedVec<CacheEntry>,
        key: AccountedVec<u8>,
        operands: [&OwnedLabels; 2],
        drop_name: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(usize, OwnedLabels), EvaluationError> {
        for (position, entry) in cache.iter().enumerate() {
            control.check()?;
            account.work(1);
            if binary_labels::equal(&entry.key, &key, control, account)? {
                return Ok((
                    position,
                    entry
                        .labels
                        .copy_checked(account, MemoryKind::Workspace, control)?,
                ));
            }
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
        let position = cache.len();
        cache.push(CacheEntry { key, labels });
        Ok((position, copy))
    }

    fn reserved(
        &self,
        signature: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<bool, Interrupted> {
        for (prior, _) in &*self.reservations {
            control.check()?;
            account.work(1);
            if *prior == signature {
                return Ok(true);
            }
        }
        Ok(false)
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
        for (position, active) in self.active[side].iter().enumerate() {
            let signature = self.signature(side, *active);
            for previous in &self.active[side][..position] {
                control.check()?;
                account.work(1);
                if self.signature(side, *previous) != signature {
                    continue;
                }
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
                for bytes in [&b"found duplicate series for the match group "[..], &group, b" on the ", side_name,
                    b" hand-side of the operation: [", &a, b", ", &b,
                    b"];many-to-many matching not allowed: matching labels must be unique on one side"] {
                    append_bytes_checked(&mut message, bytes, account, control)?;
                }
                return Err(EvaluationError::Runtime(message));
            }
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
        for active in &*self.active[side] {
            control.check()?;
            account.work(1);
            if self.signature(side, *active) == signature {
                return Ok(true);
            }
        }
        Ok(false)
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
                let present = self.present(1 - side, signature, control, account)?;
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
                let mut matched = None;
                for other in &*self.active[one] {
                    control.check()?;
                    account.work(1);
                    if self.signature(one, *other) == signature {
                        matched = Some(*other);
                        break;
                    }
                }
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
                    self.validate(control, account)?;
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
                        self.values[index] = Some(OwnedValue::Float(value));
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
                    let (ordinal, labels, value, drop_name) = match self
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
                            let pair =
                                binary_value::evaluate(self.operator, lhs, rhs, control, account)?;
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
                            for (previous, inserted) in &*self.reservations {
                                control.check()?;
                                account.work(1);
                                if *previous != signature {
                                    continue;
                                }
                                if self.matching.cardinality == Cardinality::OneToOne {
                                    return Err(EvaluationError::Invalid(
                                        "multiple matches for labels: many-to-one matching must be explicit (group_left/group_right)",
                                    ));
                                }
                                if *inserted == hash {
                                    return Err(EvaluationError::Invalid(
                                        "multiple matches for labels: grouping labels must ensure unique matches",
                                    ));
                                }
                            }
                            self.reservations.push((signature, hash));
                            if self.return_bool {
                                value = OwnedValue::Float(f64::from(pair.keep));
                            } else if !pair.keep {
                                continue;
                            }
                            (ordinal, labels, value, self.return_bool)
                        }
                    };
                    let hash = label_hash::full(&labels, control, account)?;
                    for previous in &*self.seen {
                        control.check()?;
                        account.work(1);
                        self.duplicate |= *previous == hash;
                    }
                    self.seen.push(hash);
                    return Ok(Action::Sample(EvaluatedSample {
                        ordinal,
                        labels,
                        value,
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
fn frozen_labels(
    cache: &[CacheEntry],
    key: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(usize, OwnedLabels), EvaluationError> {
    for (position, entry) in cache.iter().enumerate() {
        control.check()?;
        account.work(1);
        if binary_labels::equal(&entry.key, key, control, account)? {
            return Ok((
                position,
                entry
                    .labels
                    .copy_checked(account, MemoryKind::Workspace, control)?,
            ));
        }
    }
    Err(EvaluationError::Invalid(
        "frozen binary label cache key is missing",
    ))
}
