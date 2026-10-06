// SPDX-License-Identifier: GPL-3.0-or-later
//! Synchronous C embedding. See include/netdata-promql.h for pointer and paired-release contracts.
mod checked;
mod getters;
mod observations;
mod output;
mod provider;
#[cfg(test)]
mod tests;
pub mod types;

use crate::{
    accounting::{AllocationContext, MemoryKind, QueryAccounting, QueryObservations},
    annotations::Annotations,
    evaluator::EvaluationError,
    execution::{
        CompileError, CompiledQuery, Engine as CoreEngine, EngineOptions, ExecutionError,
        QueryRequest,
    },
    frontend::ast::ParserOptions,
    result::CollectedResult,
    shared::{Allocation, Strong},
    storage::{Control as CoreControl, Interrupted, StorageError},
};
use checked::Invalid;
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Engine {
    core: CoreEngine,
    options: EngineOptions,
}
struct ResultOwner {
    account: QueryAccounting,
    value: Option<CollectedResult>,
}
enum Outcome {
    Compile(Result<(), CompileError>),
    Execute(Result<(), ExecutionError>),
}
struct Report {
    account: QueryAccounting,
    outcome: Outcome,
    observations: QueryObservations,
    annotations: Annotations,
}
impl Report {
    fn new(account: &QueryAccounting, compile: bool) -> Strong<Self> {
        Strong::new_in(
            Self {
                account: account.clone(),
                outcome: if compile {
                    Outcome::Compile(Ok(()))
                } else {
                    Outcome::Execute(Ok(()))
                },
                observations: QueryObservations::default(),
                annotations: Annotations::default(),
            },
            Some(&AllocationContext::new(account, MemoryKind::Result)),
        )
    }
}

fn contain(operation: impl FnOnce() -> Result<u32, Invalid>) -> u32 {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => types::INVALID_ARGUMENT,
        Err(mut payload) => {
            // Payload destruction may panic; clean each replacement outside active unwinding.
            loop {
                match catch_unwind(AssertUnwindSafe(|| drop(payload))) {
                    Ok(()) => break,
                    Err(next) => payload = next,
                }
            }
            types::INTERNAL_ERROR
        }
    }
}
macro_rules! export {
    ($(fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $body:block)*) => {$ (
        #[doc = "# Safety\nAll accessed pointers/handles must satisfy the live, initialized, nonracing, correct-type and paired-release contracts in include/netdata-promql.h."]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name($($arg: $ty),*) -> u32 {
            contain(|| $body)
        }
    )*};
}
pub(super) use export;

unsafe fn borrow<'a, T, H>(value: *const H) -> Result<&'a T, Invalid> {
    let value = value.cast::<Allocation<T>>();
    checked::region(value, 1)?;
    Ok(unsafe { Strong::<T>::borrow_raw(value) })
}
unsafe fn release<T, H>(value: *mut H) -> Result<u32, Invalid> {
    let value = value.cast::<Allocation<T>>();
    checked::region(value, 1)?;
    drop(unsafe { Strong::<T>::from_raw(value) });
    Ok(types::OK)
}
enum CallControl<'a> {
    Borrowed(&'a CoreControl),
    Private(CoreControl),
}
impl std::ops::Deref for CallControl<'_> {
    type Target = CoreControl;
    fn deref(&self) -> &CoreControl {
        match self {
            Self::Borrowed(value) => value,
            Self::Private(value) => value,
        }
    }
}
unsafe fn control<'a>(
    value: *const types::Control,
    engine: &crate::accounting::EngineAccounting,
) -> Result<CallControl<'a>, Invalid> {
    if value.is_null() {
        Ok(CallControl::Private(CoreControl {
            cancellation: crate::storage::Cancellation::new_for_engine(engine),
            deadline: None,
        }))
    } else {
        Ok(CallControl::Borrowed(unsafe {
            borrow::<CoreControl, _>(value)
        }?))
    }
}
fn control_state(value: &CoreControl) -> types::ControlState {
    types::ControlState {
        status: value
            .check()
            .map_or_else(interrupted_status, |()| types::OK),
        deadline_present: u32::from(value.deadline.is_some()),
        remaining: observations::duration(
            value
                .deadline
                .map_or(std::time::Duration::ZERO, |deadline| {
                    deadline.saturating_duration_since(std::time::Instant::now())
                }),
        ),
    }
}
fn interrupted_status(value: Interrupted) -> u32 {
    match value {
        Interrupted::Cancelled => types::CANCELLED,
        Interrupted::Deadline => types::DEADLINE,
    }
}
fn compile_status(value: &Result<(), CompileError>) -> u32 {
    match value {
        Ok(()) => types::OK,
        Err(CompileError::Parse(_)) => types::PARSE_ERROR,
        Err(CompileError::Interrupted(value)) => interrupted_status(*value),
        Err(CompileError::Internal) => types::INTERNAL_ERROR,
    }
}
fn execution_status(value: &Result<(), ExecutionError>) -> u32 {
    match value {
        Ok(()) => types::OK,
        Err(ExecutionError::Internal) => types::INTERNAL_ERROR,
        Err(ExecutionError::Sink(_)) => types::OUTPUT_ERROR,
        Err(ExecutionError::Evaluation(error)) => match error {
            EvaluationError::Interrupted(value)
            | EvaluationError::Storage(StorageError::Interrupted(value)) => {
                interrupted_status(*value)
            }
            EvaluationError::Storage(StorageError::Provider(_)) => types::PROVIDER_ERROR,
            EvaluationError::Unsupported(_) => types::UNSUPPORTED,
            _ => types::EVALUATION_ERROR,
        },
    }
}
fn request(value: types::Request) -> Result<QueryRequest, Invalid> {
    if value.reserved != 0 {
        return Err(Invalid("nonzero reserved request field"));
    }
    match value.kind {
        types::REQUEST_INSTANT => Ok(QueryRequest::Instant {
            time_ms: value.start_ms,
        }),
        types::REQUEST_RANGE => Ok(QueryRequest::Range {
            start_ms: value.start_ms,
            end_ms: value.end_ms,
            step_ms: value.step_ms,
        }),
        _ => Err(Invalid("invalid request tag")),
    }
}
fn engine_options(value: types::EngineOptions) -> Result<EngineOptions, Invalid> {
    if value.flags & !types::OPTION_FLAGS != 0 || value.reserved != 0 {
        return Err(Invalid("invalid engine flags"));
    }
    Ok(EngineOptions {
        parser: ParserOptions {
            experimental_functions: value.flags & types::EXPERIMENTAL_FUNCTIONS != 0,
            binop_fill_modifiers: value.flags & types::BINOP_FILL_MODIFIERS != 0,
            extended_range_selectors: value.flags & types::EXTENDED_RANGE_SELECTORS != 0,
            duration_expressions: value.flags & types::DURATION_EXPRESSIONS != 0,
        },
        lookback_ms: value.lookback_ms,
        no_step_subquery_interval_ms: value.no_step_subquery_interval_ms,
        start_timestamps: value.flags & types::START_TIMESTAMPS != 0,
        delayed_name_removal: value.flags & types::DELAYED_NAME_REMOVAL != 0,
    })
}

export! {
    fn netdata_promql_abi_version() { Ok(types::ABI_VERSION) }
    fn netdata_promql_engine_options_default(out: *mut types::EngineOptions) {
        unsafe { checked::output(out) }?;
        let options = EngineOptions::default();
        unsafe { out.write(types::EngineOptions { header: types::Header::of::<types::EngineOptions>(), flags: types::DURATION_EXPRESSIONS, reserved: 0, lookback_ms: options.lookback_ms, no_step_subquery_interval_ms: options.no_step_subquery_interval_ms }) };
        Ok(types::OK)
    }
    fn netdata_promql_control_options_default(out: *mut types::ControlOptions) {
        unsafe { checked::output(out) }?;
        unsafe { out.write(types::ControlOptions { header: types::Header::of::<types::ControlOptions>(), ..types::ControlOptions::default() }) };
        Ok(types::OK)
    }
    fn netdata_promql_engine_create(options: *const types::EngineOptions, out: *mut *mut types::Engine) {
        unsafe { checked::output(out) }?;
        unsafe { out.write(std::ptr::null_mut()) };
        let options = if options.is_null() { EngineOptions::default() } else { engine_options(unsafe { checked::versioned(options) }?)? };
        let core = CoreEngine::default();
        let account = core.accounting.clone();
        let engine = Strong::new_for_engine(Engine { core, options }, &account);
        unsafe { out.write(engine.into_raw().cast()) };
        Ok(types::OK)
    }
    fn netdata_promql_engine_release(value: *mut types::Engine) { unsafe { release::<Engine, _>(value) } }
    fn netdata_promql_control_create(engine: *const types::Engine, options: *const types::ControlOptions, out: *mut *mut types::Control) {
        unsafe { checked::output(out) }?;
        unsafe { out.write(std::ptr::null_mut()) };
        let engine = unsafe { borrow::<Engine, _>(engine) }?;
        let options = if options.is_null() { types::ControlOptions::default() } else { unsafe { checked::versioned(options) }? };
        let deadline = if checked::present(options.deadline_present)? {
            if options.nanoseconds >= 1_000_000_000 { return Err(Invalid("invalid deadline nanoseconds")); }
            Some(std::time::Instant::now().checked_add(std::time::Duration::new(options.seconds, options.nanoseconds)).ok_or(Invalid("unrepresentable deadline"))?)
        } else { None };
        let value = Strong::new_for_engine(CoreControl { cancellation: engine.core.cancellation(), deadline }, &engine.core.accounting);
        unsafe { out.write(value.into_raw().cast()) };
        Ok(types::OK)
    }
    fn netdata_promql_control_cancel(value: *const types::Control) {
        unsafe { borrow::<CoreControl, _>(value) }?.cancellation.cancel();
        Ok(types::OK)
    }
    fn netdata_promql_control_state(value: *const types::Control, out: *mut types::ControlState) {
        unsafe { checked::output(out) }?;
        let value = control_state(unsafe { borrow::<CoreControl, _>(value) }?);
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_callback_control_state(value: *const types::CallbackControl, out: *mut types::ControlState) {
        unsafe { checked::output(out) }?;
        let value = value.cast::<CoreControl>();
        checked::region(value, 1)?;
        let value = control_state(unsafe { &*value });
        unsafe { out.write(value) };
        Ok(types::OK)
    }
    fn netdata_promql_control_release(value: *mut types::Control) { unsafe { release::<CoreControl, _>(value) } }
    fn netdata_promql_compile(engine: *const types::Engine, source: types::Bytes, raw_control: *const types::Control, out_query: *mut *mut types::Query, out_report: *mut *mut types::Report) {
        checked::distinct_outputs(out_query, out_report)?;
        unsafe { out_query.write(std::ptr::null_mut()); out_report.write(std::ptr::null_mut()); }
        let engine = unsafe { borrow::<Engine, _>(engine) }?;
        let source = unsafe { checked::bytes(source) }?;
        let control = unsafe { control(raw_control, &engine.core.accounting) }?;
        let account = engine.core.accounting.query();
        let mut report = Report::new(&account, true);
        let slot = report.get_mut().expect("unpublished compile report");
        let query = match engine.core.compile_accounted(source, engine.options, &control, &account) {
            Ok(query) => Some(Strong::new_in(query, Some(&AllocationContext::new(&account, MemoryKind::Compiled)))),
            Err(error) => { slot.outcome = Outcome::Compile(Err(error)); None },
        };
        let Outcome::Compile(outcome) = &slot.outcome else { unreachable!() };
        let status = compile_status(outcome);
        slot.observations = account.snapshot();
        drop(control);
        if let Some(query) = query { unsafe { out_query.write(query.into_raw().cast()) }; }
        unsafe { out_report.write(report.into_raw().cast()) };
        Ok(status)
    }
    fn netdata_promql_query_release(value: *mut types::Query) { unsafe { release::<CompiledQuery, _>(value) } }
    fn netdata_promql_execute(query: *const types::Query, raw_provider: *const types::Provider, raw_request: *const types::Request, raw_control: *const types::Control, raw_output: *const types::Output, out_report: *mut *mut types::Report) {
        unsafe { checked::output(out_report) }?;
        unsafe { out_report.write(std::ptr::null_mut()) };
        let mut sink = unsafe { output::Sink::new(raw_output) }?;
        let query = unsafe { borrow::<CompiledQuery, _>(query) }?;
        let request = request(unsafe { checked::versioned(raw_request) }?)?;
        let control = unsafe { control(raw_control, query.engine_accounting()) }?;
        let account = query.execution_accounting();
        let provider = unsafe { provider::Provider::new(raw_provider, &account) }?;
        let mut report = Report::new(&account, false);
        let slot = report.get_mut().expect("unpublished execution report");
        let completed = query.execute_reported_in(&provider, request, &control, &mut sink, &account);
        let status = execution_status(&completed.result);
        slot.outcome = Outcome::Execute(completed.result);
        slot.annotations = completed.annotations;
        slot.observations = completed.observations;
        unsafe { out_report.write(report.into_raw().cast()) };
        Ok(status)
    }
    fn netdata_promql_collect(query: *const types::Query, raw_provider: *const types::Provider, raw_request: *const types::Request, raw_control: *const types::Control, out_result: *mut *mut types::ResultHandle, out_report: *mut *mut types::Report) {
        checked::distinct_outputs(out_result, out_report)?;
        unsafe { out_result.write(std::ptr::null_mut()); out_report.write(std::ptr::null_mut()); }
        let query = unsafe { borrow::<CompiledQuery, _>(query) }?;
        let request = request(unsafe { checked::versioned(raw_request) }?)?;
        let control = unsafe { control(raw_control, query.engine_accounting()) }?;
        let account = query.execution_accounting();
        let provider = unsafe { provider::Provider::new(raw_provider, &account) }?;
        let mut report = Report::new(&account, false);
        let mut result = Strong::new_in(ResultOwner { account: account.clone(), value: None }, Some(&AllocationContext::new(&account, MemoryKind::Result)));
        let report_slot = report.get_mut().expect("unpublished collection report");
        let result_slot = result.get_mut().expect("unpublished collection result");
        let completed = query.collect_reported_in(&provider, request, &control, &account);
        let outcome = match completed.result { Ok(value) => { result_slot.value = Some(value); Ok(()) }, Err(error) => Err(error) };
        let status = execution_status(&outcome);
        report_slot.outcome = Outcome::Execute(outcome);
        report_slot.annotations = completed.annotations;
        let result = if status == types::OK { Some(result) } else { drop(result); None };
        report_slot.observations = account.snapshot();
        if let Some(result) = result { unsafe { out_result.write(result.into_raw().cast()) }; }
        unsafe { out_report.write(report.into_raw().cast()) };
        Ok(status)
    }
    fn netdata_promql_report_release(value: *mut types::Report) { unsafe { release::<Report, _>(value) } }
    fn netdata_promql_result_release(value: *mut types::ResultHandle) { unsafe { release::<ResultOwner, _>(value) } }
}
