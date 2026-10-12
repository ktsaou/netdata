// SPDX-License-Identifier: GPL-3.0-or-later
//! Metadata delivery owns one provisional series at a time and never opens sample cursors.
use super::*;
use crate::{
    annotations::AnnotationBuilder,
    frontend::ast::ValueType,
    result::{OutputChunk, OutputHeader, OutputSink},
    storage::{
        Provider, QueryProvider, SelectionHints, SelectionRequest, Series, SeriesSelection,
        TimeBounds, matches_checked, validate_labels_checked,
    },
    storage_observed::ObservedQuery,
};

fn enumerate(
    provider: &provider::Provider<'_>,
    bounds: TimeBounds,
    selectors: &[&selection::Selector],
    control: &CoreControl,
    sink: &mut output::Sink,
    account: &QueryAccounting,
    annotations: &mut Annotations,
) -> Result<(), ExecutionError> {
    sink.begin_checked(
        OutputHeader {
            value_type: ValueType::Vector,
            start_ms: bounds.start_ms,
            end_ms: bounds.end_ms,
            step_ms: 0,
        },
        control,
        account,
    )
    .map_err(|error| ExecutionError::output(error, account))?;
    let mut notes = AnnotationBuilder::new(account);
    let mut stopped = false;
    let mut row = 0_u64;
    let selected_result = (|| -> Result<(), ExecutionError> {
        control.check()?;
        account.provider_call();
        let query = provider
            .open(bounds, control)
            .map_err(|error| error.accounted(account))?;
        let query = ObservedQuery::new(query, account);
        for index in 0..selectors.len().max(1) {
            control.check()?;
            let selector = selectors.get(index).copied();
            let matchers = selector.map_or(&[][..], |value| value.matchers());
            account.provider_call();
            let mut selected = query.select(
                SelectionRequest {
                    matchers,
                    bounds,
                    hints: SelectionHints::default(),
                    accounting: Some(account),
                },
                control,
            )?;
            let failure = loop {
                control.check()?;
                account.provider_call();
                match selected.next(control) {
                    Ok(Some(series)) => {
                        let labels = series.label_views_checked(account, control)?;
                        validate_labels_checked(&labels, account, control)?;
                        if !matches_checked(&labels, matchers, Some(account), control)? {
                            continue;
                        }
                        let keep_going = sink
                            .metadata_row(row, &labels, control, account)
                            .map_err(|error| ExecutionError::output(error, account))?;
                        row = row.checked_add(1).ok_or(ExecutionError::Internal)?;
                        if !keep_going {
                            stopped = true;
                            break None;
                        }
                    }
                    Ok(None) => break None,
                    Err(error) => break Some(error),
                }
            };
            account.provider_call();
            for index in 0..selected.diagnostic_count() {
                if failure.is_none() {
                    control.check()?;
                }
                account.work(1);
                account.provider_call();
                selected.diagnostic(index, |note| {
                    let _view = account.hold(
                        MemoryKind::BorrowedView,
                        (size_of_val(&note) + note.message.len()) as u64,
                    );
                    notes.add_provider(
                        note,
                        selector.map_or(&[][..], |value| value.source()),
                        failure.is_none().then_some(control),
                        account,
                    )
                })?;
            }
            if let Some(error) = failure {
                return Err(error.into());
            }
            if stopped {
                break;
            }
        }
        Ok(())
    })();
    *annotations = notes.finish();
    selected_result?;
    control.check()?;
    if !annotations.is_empty() {
        sink.chunk_checked(OutputChunk::Annotations(annotations), control, account)
            .map_err(|error| ExecutionError::output(error, account))?;
    }
    Ok(())
}

export! {
    fn netdata_promql_discover(raw_engine: *const types::Engine, raw_provider: *const types::Provider,
                              bounds: types::Bounds, raw_selectors: *const *const types::Selector, selector_count: u64,
                              raw_control: *const types::Control, raw_output: *const types::Output,
                              out_report: *mut *mut types::Report) {
        unsafe { checked::output(out_report) }?;
        unsafe { out_report.write(std::ptr::null_mut()) };
        let mut sink = unsafe { output::Sink::new(raw_output) }?;
        let engine = unsafe { borrow::<Engine, _>(raw_engine) }?;
        let account = engine.core.accounting.query();
        sink.bind(&account);
        let raw = unsafe { checked::slice(raw_selectors, selector_count) }?;
        let mut selectors = crate::accounting::AccountedVec::new(&account, MemoryKind::Workspace);
        for value in raw { selectors.push(unsafe { borrow::<selection::Selector, _>(*value) }?); }
        let control = unsafe { control(raw_control, &engine.core.accounting) }?;
        let provider = unsafe { provider::Provider::new(raw_provider, &account) }?;
        let mut report = Report::new(&account, false);
        let observation = account.execution();
        let control_observation = control.observe(&account);
        let mut annotations = Annotations::default();
        let result = catch_unwind(AssertUnwindSafe(|| -> Result<(), ExecutionError> {
            let phase = account.phase(crate::statistics::Phase::Execution);
            enumerate(&provider, TimeBounds { start_ms: bounds.start_ms, end_ms: bounds.end_ms },
                      &selectors, &control, &mut sink, &account, &mut annotations)?;
            drop(phase);
            let mut observations = account.snapshot();
            observations.elapsed = observations.elapsed.saturating_add(observation.elapsed());
            sink.commit_checked(&observations, &control, &account)
                .map_err(|error| ExecutionError::output(error, &account))?;
            Ok(())
        })).unwrap_or(Err(ExecutionError::Internal));
        let result = if result.is_err() && catch_unwind(AssertUnwindSafe(|| sink.abort())).is_err() {
            Err(ExecutionError::Internal)
        } else { result };
        drop(selectors);
        drop(observation);
        drop(control_observation);
        if matches!(&result, Err(ExecutionError::Internal)) { annotations = Annotations::default(); }
        let status = execution_status(&result);
        let slot = report.get_mut().expect("unpublished discovery report");
        slot.outcome = Outcome::Execute(result);
        slot.annotations = annotations;
        slot.observations = account.snapshot();
        unsafe { out_report.write(report.into_raw().cast()) };
        Ok(status)
    }
}
