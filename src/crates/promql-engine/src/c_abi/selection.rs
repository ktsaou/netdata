// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{
    accounting::{AccountedVec, EngineAccounting},
    frontend::{
        ast::{Expr, Matcher, ParsedExpression},
        parser,
        regex::Selector as Regex,
    },
    storage::{LabelRef, matches_checked, validate_labels_checked},
};

pub(super) struct Selector {
    expression: ParsedExpression,
    account: QueryAccounting,
    engine: EngineAccounting,
}
impl Selector {
    pub(super) fn matchers(&self) -> &[Option<Matcher>] {
        let Expr::Selector(selector) = &self.expression.arena[self.expression.root].expression
        else {
            unreachable!("metric selector grammar result")
        };
        &selector.matchers
    }
    pub(super) fn source(&self) -> &[u8] {
        &self.expression.source
    }
}

export! {
    fn netdata_promql_selector_compile(raw_engine: *const types::Engine, source: types::Bytes, raw_control: *const types::Control, out: *mut *mut types::Selector, out_report: *mut *mut types::Report) {
        checked::distinct_outputs(out, out_report)?;
        unsafe { out.write(std::ptr::null_mut()); out_report.write(std::ptr::null_mut()); }
        let engine = unsafe { borrow::<Engine, _>(raw_engine) }?;
        let source = unsafe { checked::bytes(source) }?;
        let control = unsafe { control(raw_control, &engine.core.accounting) }?;
        let account = engine.core.accounting.query();
        let mut report = Report::new(&account, true);
        let timing = account.compilation();
        let observation = control.observe(&account);
        let value = (|| -> Result<Strong<Selector>, CompileError> {
            control.check().map_err(CompileError::Interrupted)?;
            let context = AllocationContext::new(&account, MemoryKind::Compiled);
            let parsed = parser::parse_metric_selector_in(source, engine.options.parser,
                |pattern| Regex::compile_checked(pattern, Some(&context), Some(&control)), Some(&account), Some(&control))
                .map_err(CompileError::Interrupted)?;
            if !parsed.errors.is_empty() { return Err(CompileError::Parse(parsed.errors)); }
            let mut owned = AccountedVec::with_capacity_in(source.len(), Some(&context));
            crate::result::append_bytes_checked(&mut owned, source, &account, &control).map_err(CompileError::Interrupted)?;
            Ok(Strong::new_in(Selector { expression: ParsedExpression { source: owned, arena: parsed.arena, root: parsed.root.expect("valid selector root") }, account: account.clone(), engine: engine.core.accounting.clone() }, Some(&context)))
        })();
        drop(observation);
        drop(timing);
        let outcome = match value {
            Ok(selector) => { unsafe { out.write(selector.into_raw().cast()) }; Ok(()) },
            Err(error) => Err(error),
        };
        let status = compile_status(&outcome);
        let slot = report.get_mut().expect("unpublished selector report");
        slot.outcome = Outcome::Compile(outcome);
        slot.observations = account.snapshot();
        unsafe { out_report.write(report.into_raw().cast()) };
        Ok(status)
    }
    fn netdata_promql_selector_release(value: *mut types::Selector) { unsafe { release::<Selector, _>(value) } }
    fn netdata_promql_selector_matcher_count(value: *const types::Selector, out: *mut u64) {
        unsafe { checked::output(out) }?;
        let selector = unsafe { borrow::<Selector, _>(value) }?;
        unsafe { out.write(selector.matchers().iter().flatten().count() as u64) };
        Ok(types::OK)
    }
    fn netdata_promql_selector_matcher(value: *const types::Selector, index: u64, out: *mut types::Matcher) {
        unsafe { checked::output(out) }?;
        let selector = unsafe { borrow::<Selector, _>(value) }?;
        let matcher = selector.matchers().iter().flatten().nth(usize::try_from(index).map_err(|_| Invalid("matcher index overflow"))?).ok_or(Invalid("matcher index out of bounds"))?;
        unsafe { out.write(types::Matcher { kind: matcher.kind as u32, reserved: 0, name: types::Bytes::of(&matcher.name), value: types::Bytes::of(&matcher.value) }) };
        Ok(types::OK)
    }
    fn netdata_promql_selector_nonempty(value: *const types::Selector, raw_control: *const types::Control, out: *mut u32) {
        unsafe { checked::output(out) }?;
        unsafe { out.write(0) };
        let selector = unsafe { borrow::<Selector, _>(value) }?;
        let control = unsafe { control(raw_control, &selector.engine) }?;
        let _observation = control.observe(&selector.account);
        for matcher in selector.matchers().iter().flatten() {
            match matcher.matches_with(b"", Some(&selector.account), Some(&control)) {
                Ok(false) => { unsafe { out.write(1) }; return Ok(types::OK); },
                Ok(true) => (),
                Err(error) => return Ok(interrupted_status(error)),
            }
        }
        control.check().map_or_else(|error| Ok(interrupted_status(error)), |()| Ok(types::OK))
    }
    fn netdata_promql_selector_matches(value: *const types::Selector, labels: types::Labels, raw_control: *const types::Control, out: *mut u32) {
        unsafe { checked::output(out) }?;
        unsafe { out.write(0) };
        let selector = unsafe { borrow::<Selector, _>(value) }?;
        let control = unsafe { control(raw_control, &selector.engine) }?;
        let _observation = control.observe(&selector.account);
        let raw = unsafe { checked::slice(labels.data, labels.len) }?;
        let mut borrowed = selector.account.hold(MemoryKind::BorrowedView, size_of_val(raw) as u64);
        if let Err(error) = control.check() { return Ok(interrupted_status(error)); }
        let mut footprint = checked::Footprint::new(&selector.account);
        footprint.add(labels.data, labels.len)?;
        let context = AllocationContext::new(&selector.account, MemoryKind::Workspace);
        let mut values = AccountedVec::with_capacity_in(raw.len(), Some(&context));
        let mut status = types::OK;
        for label in raw {
            if let Err(error) = control.check() { status = interrupted_status(error); break; }
            selector.account.work(1);
            footprint.add(label.name.data, label.name.len)?;
            footprint.add(label.value.data, label.value.len)?;
            values.push(LabelRef { name: unsafe { checked::bytes(label.name) }?, value: unsafe { checked::bytes(label.value) }? });
        }
        borrowed.resize(footprint.bytes()?);
        if status != types::OK { return Ok(status); }
        match validate_labels_checked(&values, &selector.account, &control) {
            Ok(()) => (),
            Err(StorageError::Interrupted(error)) => return Ok(interrupted_status(error)),
            Err(_) => return Err(Invalid("invalid selector label set")),
        }
        match matches_checked(&values, selector.matchers(), Some(&selector.account), &control) {
            Ok(value) => { unsafe { out.write(u32::from(value)) }; Ok(types::OK) },
            Err(error) => Ok(interrupted_status(error)),
        }
    }
}
