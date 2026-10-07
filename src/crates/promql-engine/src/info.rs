// SPDX-License-Identifier: GPL-3.0-or-later
// INFO selection, signatures and joins derive from pinned promql/info.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, AllocationContext, MemoryKind, QueryAccounting},
    annotations::AnnotationBuilder,
    binary_labels,
    evaluator::{EvaluatedSample, EvaluationError, EvaluationOptions},
    evaluator_order,
    evaluator_roster::Descriptor,
    frontend::{
        ast::{Expr, MatchType, Matcher, NodeId, ParsedExpression},
        number,
        regex::Selector,
    },
    instant_select, label_hash,
    query_catalog::{Catalog, InfoSite},
    query_preparation::{Prepared, SelectorReference},
    result::{OwnedLabels, append_bytes_checked},
    storage::{
        Control, Interrupted, QueryProvider, SelectionHints, SelectionRequest, Series,
        StorageError, TimeBounds,
    },
    value::SampleValueRef,
};
use std::{cell::RefCell, fmt::Write};

const NAME: &[u8] = b"__name__";
const IDS: [&[u8]; 2] = [b"instance", b"job"];

struct InfoRow {
    site: InfoSite,
    ordinal: usize,
    labels: OwnedLabels,
    hash: u64,
    signature: AccountedVec<u8>,
}
struct Binding {
    hash: u64,
    labels: OwnedLabels,
    drop_name: bool,
}
pub(crate) struct Cache {
    bindings: AccountedVec<Binding>,
}
impl Cache {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) struct Preparation<'a> {
    pub(crate) node: NodeId,
    pub(crate) cache: &'a RefCell<Cache>,
}
pub(crate) struct Plan {
    rows: AccountedVec<Descriptor>,
    base_owners: AccountedVec<usize>,
    ignore: AccountedVec<u64>,
    info: AccountedVec<InfoRow>,
    names: AccountedVec<AccountedVec<u8>>,
    data_matchers: AccountedVec<Option<Matcher>>,
    reference: SelectorReference,
    bindings: AccountedVec<Binding>,
}
impl Plan {
    pub(crate) fn prepare<Q: QueryProvider>(
        rows: AccountedVec<Descriptor>,
        call: (NodeId, &ParsedExpression, &Prepared),
        catalog: &Catalog<'_, Q>,
        scope: (EvaluationOptions, TimeBounds),
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(Self, AnnotationBuilder), EvaluationError> {
        let (node, expression, prepared) = call;
        let (scope, data_bounds) = scope;
        let Expr::Call { arguments, .. } = &expression.arena[node].expression else {
            unreachable!("INFO call")
        };
        let reference = prepared.info_reference(expression, arguments[0], control, account)?;
        let mut result = Self {
            rows,
            base_owners: AccountedVec::new(account, MemoryKind::Workspace),
            ignore: AccountedVec::new(account, MemoryKind::Workspace),
            info: AccountedVec::new(account, MemoryKind::Workspace),
            names: AccountedVec::new(account, MemoryKind::Workspace),
            data_matchers: AccountedVec::new(account, MemoryKind::Workspace),
            reference,
            bindings: AccountedVec::new(account, MemoryKind::Workspace),
        };
        let mut name_matchers = AccountedVec::new(account, MemoryKind::Workspace);
        if let Some(selector) = arguments.get(1) {
            let Expr::Selector(selector) = &expression.arena[*selector].expression else {
                unreachable!("INFO label selector")
            };
            for matcher in selector.matchers.iter().flatten() {
                control.check()?;
                account.work(1);
                let copy = Some(matcher.copy_for(Some(account), MemoryKind::Workspace, control)?);
                if binary_labels::equal(&matcher.name, NAME, control, account)? {
                    name_matchers.push(copy);
                } else {
                    result.data_matchers.push(copy);
                }
            }
        }
        let positive = name_matchers
            .iter()
            .flatten()
            .any(|matcher| matches!(matcher.kind, MatchType::Equal | MatchType::Regex));
        if !positive {
            let (kind, value) = if name_matchers.is_empty() {
                (MatchType::Equal, &b"target_info"[..])
            } else {
                (MatchType::Regex, &b".+_info"[..])
            };
            name_matchers.insert(0, Some(matcher(NAME, value, kind, control, account)?));
        }
        for (position, row) in result.rows.iter().enumerate() {
            let name = row.labels.get_checked(NAME, account, control)?;
            let mut ignore = true;
            for matcher in name_matchers.iter().flatten() {
                ignore &= matcher.matches_with(name, Some(account), Some(control))?;
            }
            if ignore {
                result.ignore.push(row.hash);
            }
            let mut owner = position;
            for (candidate, later) in result.rows.iter().enumerate().skip(position + 1) {
                control.check()?;
                account.work(1);
                if later.hash == row.hash {
                    owner = candidate;
                }
            }
            result.base_owners.push(owner);
        }
        let mut sites = AccountedVec::new(account, MemoryKind::Workspace);
        let mut notes = AnnotationBuilder::new(account);
        for presence in 1..=3 {
            let mut values: [AccountedVec<AccountedVec<u8>>; 2] =
                std::array::from_fn(|_| AccountedVec::new(account, MemoryKind::Workspace));
            let mut found = false;
            for row in &*result.rows {
                control.check()?;
                account.work(1);
                if ignored(row.hash, &result.ignore, control, account)? {
                    continue;
                }
                let instance = row.labels.get_checked(IDS[0], account, control)?;
                let job = row.labels.get_checked(IDS[1], account, control)?;
                let bits = u8::from(!job.is_empty()) | (u8::from(!instance.is_empty()) << 1);
                if bits != presence {
                    continue;
                }
                found = true;
                for (set, value) in values.iter_mut().zip([instance, job]) {
                    if !value.is_empty() && !binary_labels::named(value, set, control, account)? {
                        let mut copy = AccountedVec::new(account, MemoryKind::Workspace);
                        append_bytes_checked(&mut copy, value, account, control)?;
                        set.push(copy);
                    }
                }
            }
            if !found {
                continue;
            }
            let mut matchers = AccountedVec::new(account, MemoryKind::Workspace);
            for (name, values) in IDS.into_iter().zip(&mut values) {
                evaluator_order::sort_by_with(values, Some(control), Some(account), |a, b| {
                    binary_labels::compare(a, b, control, account)
                })?;
                let mut pattern = AccountedVec::new(account, MemoryKind::Workspace);
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        pattern.push(b'|');
                    }
                    quote_meta(&mut pattern, value, control, account)?;
                }
                let kind = if values.is_empty() {
                    MatchType::Equal
                } else {
                    MatchType::Regex
                };
                matchers.push(Some(matcher(name, &pattern, kind, control, account)?));
            }
            for matcher in result
                .data_matchers
                .iter()
                .chain(name_matchers.iter())
                .flatten()
            {
                matchers.push(Some(matcher.copy_for(
                    Some(account),
                    MemoryKind::Workspace,
                    control,
                )?));
            }
            let offset_ms = reference.offset_ns / 1_000_000;
            let bounds = TimeBounds {
                start_ms: reference
                    .timestamp_ms
                    .unwrap_or(scope.start_ms)
                    .wrapping_sub(scope.lookback_ms)
                    .wrapping_add(1)
                    .wrapping_sub(offset_ms),
                end_ms: reference
                    .timestamp_ms
                    .unwrap_or(scope.end_ms)
                    .wrapping_sub(offset_ms),
            };
            let (site, warnings) = catalog.select_info(
                SelectionRequest {
                    matchers: &matchers,
                    bounds,
                    hints: SelectionHints {
                        step_ms: scope.step_ms.max(1),
                        function: "info",
                        ..SelectionHints::default()
                    },
                    accounting: Some(account),
                },
                control,
                account,
            )?;
            sites.push(site);
            notes.merge(warnings, &expression.source);
        }
        // All selections expand before the source's series-major raw evaluation starts.
        for site in &*sites {
            for ordinal in 0..catalog.info_len(*site) {
                let row = catalog
                    .with_info_series(*site, ordinal, |series| {
                        let views = series.label_views_checked(account, control)?;
                        let labels = OwnedLabels::copy_from_checked(
                            &views,
                            account,
                            MemoryKind::Workspace,
                            control,
                        )?;
                        let mut time_ms = scope.start_ms;
                        let mut productive = false;
                        loop {
                            if let Some((_, histogram)) = read(
                                series,
                                time_ms,
                                reference,
                                scope.lookback_ms,
                                data_bounds,
                                control,
                                account,
                            )? {
                                if histogram {
                                    let rendered =
                                        binary_labels::render(&labels, control, account)?;
                                    let mut error =
                                        AccountedVec::new(account, MemoryKind::Workspace);
                                    append_bytes_checked(
                                        &mut error,
                                        b"this should be an info metric, with float samples: ",
                                        account,
                                        control,
                                    )?;
                                    append_bytes_checked(&mut error, &rendered, account, control)?;
                                    return Err(EvaluationError::Runtime(error));
                                }
                                productive = true;
                            }
                            let Some(next) = time_ms
                                .checked_add(scope.step_ms)
                                .filter(|next| *next > time_ms && *next <= scope.end_ms)
                            else {
                                break;
                            };
                            time_ms = next;
                        }
                        let hash = label_hash::full(&labels, control, account)?;
                        Ok(productive.then_some(InfoRow {
                            site: *site,
                            ordinal,
                            labels,
                            hash,
                            signature: AccountedVec::new(account, MemoryKind::Workspace),
                        }))
                    })
                    .expect("INFO series ordinal")?;
                if let Some(row) = row {
                    result.info.push(row);
                }
            }
        }
        for position in 0..result.info.len() {
            let mut owner = position;
            for candidate in position + 1..result.info.len() {
                control.check()?;
                account.work(1);
                if result.info[candidate].hash == result.info[position].hash {
                    owner = candidate;
                }
            }
            let row = &result.info[owner];
            let name = row.labels.get_checked(NAME, account, control)?;
            let key = signature(&row.labels, name, control, account)?;
            result.info[position].signature = key;
            let name = result.info[position]
                .labels
                .get_checked(NAME, account, control)?;
            if !binary_labels::named(name, &result.names, control, account)? {
                let mut copy = AccountedVec::new(account, MemoryKind::Workspace);
                append_bytes_checked(&mut copy, name, account, control)?;
                result.names.push(copy);
            }
        }
        Ok((result, notes))
    }
    pub(crate) fn finish(&mut self, cache: Cache) {
        self.bindings = cache.bindings;
    }
    fn row(
        &self,
        sample: &EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<usize, EvaluationError> {
        for (position, row) in self.rows.iter().enumerate() {
            control.check()?;
            account.work(1);
            if row.ordinal == sample.ordinal
                && binary_labels::same(&row.labels, &sample.labels, control, account)?
            {
                return Ok(position);
            }
        }
        Err(EvaluationError::Invalid("INFO base replay row is missing"))
    }
}

fn ignored(
    hash: u64,
    values: &[u64],
    control: &Control,
    account: &QueryAccounting,
) -> Result<bool, Interrupted> {
    for value in values {
        control.check()?;
        account.work(1);
        if *value == hash {
            return Ok(true);
        }
    }
    Ok(false)
}

fn matcher(
    name: &[u8],
    value: &[u8],
    kind: MatchType,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Matcher, EvaluationError> {
    let context = AllocationContext::new(account, MemoryKind::Workspace);
    let compiled = if kind == MatchType::Regex {
        Some(
            Selector::compile_checked(value, Some(&context), Some(control))?
                .map_err(EvaluationError::Regex)?,
        )
    } else {
        None
    };
    let mut owned_name = AccountedVec::new(account, MemoryKind::Workspace);
    let mut owned_value = AccountedVec::new(account, MemoryKind::Workspace);
    append_bytes_checked(&mut owned_name, name, account, control)?;
    append_bytes_checked(&mut owned_value, value, account, control)?;
    Ok(Matcher {
        kind,
        name: owned_name,
        value: owned_value,
        compiled,
    })
}

fn quote_meta(
    target: &mut AccountedVec<u8>,
    value: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    for bytes in value.chunks(1024) {
        control.check()?;
        account.work(bytes.len() as u64);
        for byte in bytes {
            if b"\\.+*?()|[]{}^$".contains(byte) {
                target.push(b'\\');
            }
            target.push(*byte);
        }
    }
    Ok(())
}

fn signature(
    labels: &OwnedLabels,
    name: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<AccountedVec<u8>, Interrupted> {
    let mut selected = OwnedLabels::new(account, MemoryKind::Workspace);
    // ScratchBuilder keeps the explicitly injected name, including an empty name value.
    let mut metric_name = AccountedVec::new(account, MemoryKind::Workspace);
    let mut metric_value = AccountedVec::new(account, MemoryKind::Workspace);
    append_bytes_checked(&mut metric_name, NAME, account, control)?;
    append_bytes_checked(&mut metric_value, name, account, control)?;
    selected.0.push(crate::result::OwnedLabel {
        name: metric_name,
        value: metric_value,
    });
    for id in IDS {
        let value = labels.get_checked(id, account, control)?;
        selected.set(id, value, account, MemoryKind::Workspace, control)?;
    }
    let mut result = AccountedVec::new(account, MemoryKind::Workspace);
    binary_labels::append(&mut result, &selected, None, control, account)?;
    Ok(result)
}

fn read<S: Series>(
    series: &S,
    time_ms: i64,
    reference: SelectorReference,
    lookback_ms: i64,
    data_bounds: TimeBounds,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<(i64, bool)>, EvaluationError> {
    let lookup = reference
        .timestamp_ms
        .unwrap_or(time_ms)
        .wrapping_sub(reference.offset_ns / 1_000_000);
    Ok(instant_select::select::<_, _, StorageError>(
        series,
        lookup,
        lookback_ms,
        data_bounds,
        control,
        account,
        |value| Ok(!matches!(value, SampleValueRef::Float(_))),
    )?
    .map(|(time, _, histogram)| (time, histogram)))
}

#[derive(Clone, Copy)]
struct Active {
    row: usize,
    kind: u8,
}
struct Winner {
    row: usize,
    timestamp: i64,
}
enum Phase {
    Start,
    Base,
    Join,
    Choose,
    Output,
    Finished,
}
pub(crate) enum Action<'a> {
    Child(Option<&'a usize>),
    Sample(EvaluatedSample),
    Done,
}
pub(crate) struct Step<'a> {
    plan: &'a Plan,
    phase: Phase,
    active: AccountedVec<Active>,
    winners: AccountedVec<Winner>,
    matches: AccountedVec<usize>,
    seen: AccountedVec<u64>,
    duplicate: bool,
    position: usize,
    pending: Option<EvaluatedSample>,
}
impl<'a> Step<'a> {
    pub(crate) fn new(plan: &'a Plan, account: &QueryAccounting) -> Self {
        Self {
            plan,
            phase: Phase::Start,
            active: AccountedVec::new(account, MemoryKind::Workspace),
            winners: AccountedVec::new(account, MemoryKind::Workspace),
            matches: AccountedVec::new(account, MemoryKind::Workspace),
            seen: AccountedVec::new(account, MemoryKind::Workspace),
            duplicate: false,
            position: 0,
            pending: None,
        }
    }
    pub(crate) fn receive(
        &mut self,
        sample: EvaluatedSample,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        let row = self.plan.row(&sample, control, account)?;
        match self.phase {
            Phase::Base => self.active.push(Active {
                row,
                kind: sample.value.kind_bit(),
            }),
            Phase::Output => {
                let active = self.active[self.position];
                if row == active.row && sample.value.kind_bit() == active.kind {
                    self.pending = Some(sample);
                }
            }
            _ => return Err(EvaluationError::Invalid("unexpected INFO child sample")),
        }
        Ok(())
    }
    pub(crate) fn advance<Q: QueryProvider>(
        &mut self,
        node: NodeId,
        options: EvaluationOptions,
        storage: (&Catalog<'_, Q>, TimeBounds),
        preparation: Option<Preparation>,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Action<'a>, EvaluationError> {
        let (catalog, data_bounds) = storage;
        loop {
            control.check()?;
            account.work(1);
            match self.phase {
                Phase::Start => {
                    self.phase = Phase::Base;
                    return Ok(Action::Child(None));
                }
                Phase::Base => {
                    self.phase = if self.active.is_empty() {
                        Phase::Finished
                    } else {
                        Phase::Join
                    };
                }
                Phase::Join => {
                    for (position, row) in self.plan.info.iter().enumerate() {
                        let value = catalog
                            .with_info_series(row.site, row.ordinal, |series| {
                                read(
                                    series,
                                    options.time_ms,
                                    self.plan.reference,
                                    options.lookback_ms,
                                    data_bounds,
                                    control,
                                    account,
                                )
                            })
                            .expect("INFO series ordinal")?;
                        let Some((timestamp, histogram)) = value else {
                            continue;
                        };
                        if histogram {
                            return Err(EvaluationError::Invalid("info sample should be float"));
                        }
                        let timestamp = number::go_i64(timestamp as f64);
                        let mut found = None;
                        for (index, previous) in self.winners.iter().enumerate() {
                            if binary_labels::equal(
                                &self.plan.info[previous.row].signature,
                                &row.signature,
                                control,
                                account,
                            )? {
                                found = Some(index);
                                break;
                            }
                        }
                        if let Some(index) = found {
                            let previous = &mut self.winners[index];
                            if previous.timestamp == timestamp {
                                let existing = binary_labels::render(
                                    &self.plan.info[previous.row].labels,
                                    control,
                                    account,
                                )?;
                                let new = binary_labels::render(&row.labels, control, account)?;
                                let mut message = AccountedVec::new(account, MemoryKind::Workspace);
                                append_bytes_checked(
                                    &mut message,
                                    b"found duplicate series for info metric: existing ",
                                    account,
                                    control,
                                )?;
                                append_bytes_checked(&mut message, &existing, account, control)?;
                                write!(message, " @ {}, new ", previous.timestamp)
                                    .expect("accounted byte writer");
                                append_bytes_checked(&mut message, &new, account, control)?;
                                write!(message, " @ {timestamp}").expect("accounted byte writer");
                                account.work(64);
                                return Err(EvaluationError::Runtime(message));
                            }
                            if previous.timestamp < timestamp {
                                *previous = Winner {
                                    row: position,
                                    timestamp,
                                };
                            }
                        } else {
                            self.winners.push(Winner {
                                row: position,
                                timestamp,
                            });
                        }
                    }
                    self.phase = Phase::Choose;
                }
                Phase::Choose => {
                    if self.position == self.active.len() {
                        if self.duplicate {
                            return Err(EvaluationError::Invalid(
                                "vector cannot contain metrics with the same labelset",
                            ));
                        }
                        self.phase = Phase::Finished;
                        continue;
                    }
                    self.phase = Phase::Output;
                    let row = &self.plan.rows[self.active[self.position].row];
                    return Ok(Action::Child(Some(&row.ordinal)));
                }
                Phase::Output => {
                    self.phase = Phase::Choose;
                    self.position += 1;
                    let Some(mut sample) = self.pending.take() else {
                        return Err(EvaluationError::Invalid(
                            "INFO base replay sample is missing",
                        ));
                    };
                    let row = self.active[self.position - 1].row;
                    let Some(labels) = self.enrich(row, control, account)? else {
                        continue;
                    };
                    let hash = label_hash::full(&labels, control, account)?;
                    self.duplicate |= ignored(hash, &self.seen, control, account)?;
                    self.seen.push(hash);
                    if let Some(preparation) = preparation.filter(|value| value.node == node) {
                        let mut cache = preparation.cache.borrow_mut();
                        let ordinal = binding(&cache.bindings, hash, control, account)?;
                        let ordinal = if let Some(ordinal) = ordinal {
                            ordinal
                        } else {
                            let ordinal = cache.bindings.len();
                            cache.bindings.push(Binding {
                                hash,
                                labels: labels.copy_checked(
                                    account,
                                    MemoryKind::Workspace,
                                    control,
                                )?,
                                drop_name: sample.drop_name,
                            });
                            ordinal
                        };
                        sample.ordinal = ordinal;
                        sample.labels = cache.bindings[ordinal].labels.copy_checked(
                            account,
                            MemoryKind::Workspace,
                            control,
                        )?;
                        sample.drop_name = cache.bindings[ordinal].drop_name;
                    } else {
                        let ordinal = binding(&self.plan.bindings, hash, control, account)?
                            .ok_or(EvaluationError::Invalid("INFO output binding is missing"))?;
                        sample.ordinal = ordinal;
                        sample.labels = self.plan.bindings[ordinal].labels.copy_checked(
                            account,
                            MemoryKind::Workspace,
                            control,
                        )?;
                        sample.drop_name = self.plan.bindings[ordinal].drop_name;
                    }
                    sample.source_time_ms = options.time_ms;
                    sample.source_start_time_ms = 0;
                    return Ok(Action::Sample(sample));
                }
                Phase::Finished => return Ok(Action::Done),
            }
        }
    }

    fn enrich(
        &mut self,
        row: usize,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<OwnedLabels>, EvaluationError> {
        let base = &self.plan.rows[row];
        if ignored(base.hash, &self.plan.ignore, control, account)? {
            return Ok(Some(base.labels.copy_checked(
                account,
                MemoryKind::Workspace,
                control,
            )?));
        }
        let owner = &self.plan.rows[self.plan.base_owners[row]].labels;
        self.matches.clear();
        for name in &*self.plan.names {
            let signature = signature(owner, name, control, account)?;
            for winner in &*self.winners {
                if binary_labels::equal(
                    &signature,
                    &self.plan.info[winner.row].signature,
                    control,
                    account,
                )? {
                    self.matches.push(winner.row);
                    break;
                }
            }
        }
        evaluator_order::sort_by_with(&mut self.matches, Some(control), Some(account), |a, b| {
            self.plan.info[*a]
                .labels
                .compare_checked(&self.plan.info[*b].labels, account, control)
        })?;
        let mut added = OwnedLabels::new(account, MemoryKind::Workspace);
        for matched in &*self.matches {
            for label in &*self.plan.info[*matched].labels.0 {
                control.check()?;
                account.work(1);
                if binary_labels::equal(&label.name, NAME, control, account)? {
                    continue;
                }
                if !self.plan.data_matchers.is_empty() {
                    let mut selected = false;
                    for matcher in self.plan.data_matchers.iter().flatten() {
                        selected |=
                            binary_labels::equal(&matcher.name, &label.name, control, account)?;
                    }
                    if !selected {
                        continue;
                    }
                }
                let value = added.get_checked(&label.name, account, control)?;
                if !value.is_empty()
                    && !binary_labels::equal(value, &label.value, control, account)?
                {
                    let mut message = AccountedVec::new(account, MemoryKind::Workspace);
                    append_bytes_checked(&mut message, b"conflicting label: ", account, control)?;
                    append_bytes_checked(&mut message, &label.name, account, control)?;
                    return Err(EvaluationError::Runtime(message));
                }
                if base.labels.position(&label.name, account, control)?.is_ok() {
                    continue;
                }
                added.set(
                    &label.name,
                    &label.value,
                    account,
                    MemoryKind::Workspace,
                    control,
                )?;
            }
        }
        if self.matches.is_empty() {
            for matcher in self.plan.data_matchers.iter().flatten() {
                if !matcher.matches_with(b"", Some(account), Some(control))? {
                    return Ok(None);
                }
            }
        }
        let mut labels = base
            .labels
            .copy_checked(account, MemoryKind::Workspace, control)?;
        labels.reset_checked(account, control)?;
        for label in &*added.0 {
            labels.set(
                &label.name,
                &label.value,
                account,
                MemoryKind::Workspace,
                control,
            )?;
        }
        Ok(Some(labels))
    }
}

fn binding(
    bindings: &[Binding],
    hash: u64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Option<usize>, Interrupted> {
    for (position, binding) in bindings.iter().enumerate() {
        control.check()?;
        account.work(1);
        if binding.hash == hash {
            return Ok(Some(position));
        }
    }
    Ok(None)
}
