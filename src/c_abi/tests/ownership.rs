// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
fn compilation_reports_preserve_native_order_spans_and_owner_lifetimes() {
    for source in [
        b"sum(m) by (a, a) + unknown(m)".as_slice(),
        b"{x=~\"[\"}",
        b"m +",
    ] {
        let reference = CoreEngine::default();
        let Err(CompileError::Parse(expected)) =
            reference.compile(source, EngineOptions::default(), &CoreControl::default())
        else {
            panic!("parse failure fixture");
        };
        unsafe {
            let engine = engine();
            let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let mut query = ptr::null_mut();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_compile(
                    engine,
                    types::Bytes::of(source),
                    ptr::null(),
                    &mut query,
                    &mut report
                ),
                types::PARSE_ERROR
            );
            assert!(query.is_null());
            let mut count = 0;
            assert_eq!(
                netdata_promql_report_error_count(report, &mut count),
                types::OK
            );
            assert_eq!(count, expected.len() as u64);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            for (offset, expected) in expected.iter().enumerate() {
                let mut error = types::Error::default();
                assert_eq!(
                    netdata_promql_report_error(report, offset as u64, &mut error),
                    types::OK
                );
                assert_eq!(
                    (
                        error.kind,
                        error.position_present,
                        error.position.start,
                        error.position.end
                    ),
                    (1, 1, expected.span.start as u64, expected.span.end as u64)
                );
                assert_eq!(checked::bytes(error.message).unwrap(), &*expected.message);
            }
            let mut error = types::Error::default();
            assert_eq!(
                netdata_promql_report_error(report, count, &mut error),
                types::INVALID_ARGUMENT
            );
            let mut observed = types::QueryObservations::default();
            assert_eq!(
                netdata_promql_report_live_observations(report, &mut observed),
                types::OK
            );
            assert!(observed.memory[MemoryKind::Compiled as usize].current > 0);
            assert_eq!(observed.memory[MemoryKind::Workspace as usize].current, 0);
            assert_eq!(accounting.snapshot().completed_executions, 0);
            assert_eq!(accounting.snapshot().elapsed, std::time::Duration::ZERO);
            assert_eq!(netdata_promql_report_release(report), types::OK);
            accounting.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn owned_strings_and_completion_observations_survive_source_handle_release() {
    unsafe {
        let engine = engine();
        let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut source = b"\"a\\x00\\xffz\"".to_vec();
        let query = compiled(engine, &source);
        source.fill(b'X');
        drop(source);
        let mut query_source = types::Bytes::default();
        assert_eq!(
            netdata_promql_query_source(query, &mut query_source),
            types::OK
        );
        assert_eq!(checked::bytes(query_source).unwrap(), b"\"a\\x00\\xffz\"");
        let mut fixture = Fixture::new(1);
        let table = fixture.table();
        let mut result = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect(
                query,
                &table,
                &instant(-17),
                ptr::null(),
                &mut result,
                &mut report
            ),
            types::OK
        );
        let mut completion = types::QueryObservations::default();
        assert_eq!(
            netdata_promql_report_observations(report, &mut completion),
            types::OK
        );
        assert!(completion.memory[MemoryKind::Result as usize].current > 0);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        drop(fixture);
        let mut string = types::StringValue::default();
        assert_eq!(netdata_promql_result_string(result, &mut string), types::OK);
        assert_eq!(string.time_ms, -17);
        assert_eq!(checked::bytes(string.value).unwrap(), b"a\0\xffz");
        assert_eq!(netdata_promql_result_release(result), types::OK);
        let mut fixed = types::QueryObservations::default();
        let mut live = types::QueryObservations::default();
        assert_eq!(
            netdata_promql_report_observations(report, &mut fixed),
            types::OK
        );
        assert_eq!(
            netdata_promql_report_live_observations(report, &mut live),
            types::OK
        );
        assert_eq!(
            fixed.memory[MemoryKind::Result as usize].current,
            completion.memory[MemoryKind::Result as usize].current
        );
        assert!(
            live.memory[MemoryKind::Result as usize].current
                < fixed.memory[MemoryKind::Result as usize].current
        );
        assert_eq!(netdata_promql_report_release(report), types::OK);
        accounting.snapshot().assert_memory_released();
    }
}

struct SharedPointer<T>(*const T);
// These tests share immutable handles; their final paired releases follow every worker join.
unsafe impl<T: Send + Sync> Sync for SharedPointer<T> {}
impl<T> SharedPointer<T> {
    fn get(&self) -> *const T {
        self.0
    }
}

#[test]
fn concurrent_c_queries_share_a_plan_and_keep_the_cancellation_creator_owner() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Engine>();
    send_sync::<CompiledQuery>();
    send_sync::<CoreControl>();
    unsafe {
        let engine = engine();
        let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let creator = self::engine();
        let creator_accounting = borrow::<Engine, _>(creator)
            .unwrap()
            .core
            .accounting
            .clone();
        let query = compiled(engine, b"m");
        let mut token = ptr::null_mut();
        assert_eq!(
            netdata_promql_control_create(creator, ptr::null(), &mut token),
            types::OK
        );
        let creator_bytes =
            creator_accounting.snapshot().memory[MemoryKind::Accounting as usize].current;
        assert_eq!(netdata_promql_engine_release(creator), types::OK);
        assert!(
            creator_accounting.snapshot().memory[MemoryKind::Accounting as usize].current
                < creator_bytes
        );
        let retained_token_bytes =
            creator_accounting.snapshot().memory[MemoryKind::Accounting as usize].current;
        let query = SharedPointer(query);
        let token = SharedPointer(token);
        let gates = std::sync::Arc::new((std::sync::Barrier::new(5), std::sync::Barrier::new(5)));
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in 0..4 {
                let (query, token, gates) = (&query, &token, gates.clone());
                workers.push(scope.spawn(move || {
                    let mut fixture = Fixture::new(1);
                    fixture.gates = Some(gates);
                    let table = fixture.table();
                    let mut result = ptr::null_mut();
                    let mut report = ptr::null_mut();
                    let status = netdata_promql_collect(
                        query.get(),
                        &table,
                        &instant(1000),
                        token.get(),
                        &mut result,
                        &mut report,
                    );
                    assert_eq!(status, types::CANCELLED);
                    assert!(result.is_null());
                    for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                        assert_eq!(opened.get(), closed.get());
                    }
                    assert_eq!(fixture.opened[0].get(), 1);
                    let mut observed = types::QueryObservations::default();
                    assert_eq!(
                        netdata_promql_report_observations(report, &mut observed),
                        types::OK
                    );
                    assert_eq!(observed.cancellation_references.external.peak, 0);
                    assert!(observed.cancellation_references.engine_owned.peak > 0);
                    assert_eq!(observed.cancellation_references.engine_owned.current, 0);
                    assert_eq!(netdata_promql_report_release(report), types::OK);
                }));
            }
            gates.0.wait();
            let mut observed = types::EngineObservations::default();
            assert_eq!(
                netdata_promql_engine_observations(engine, &mut observed),
                types::OK
            );
            assert_eq!(
                (observed.active_executions, observed.peak_executions),
                (4, 4)
            );
            assert_eq!(
                creator_accounting.snapshot().memory[MemoryKind::Accounting as usize].current,
                retained_token_bytes
            );
            assert_eq!(netdata_promql_control_cancel(token.get()), types::OK);
            gates.1.wait();
            for worker in workers {
                worker.join().unwrap();
            }
        });
        let mut observed = types::EngineObservations::default();
        assert_eq!(
            netdata_promql_engine_observations(engine, &mut observed),
            types::OK
        );
        assert_eq!(
            (observed.active_executions, observed.completed_executions),
            (0, 4)
        );
        assert_eq!(
            netdata_promql_control_release(token.get().cast_mut()),
            types::OK
        );
        creator_accounting.snapshot().assert_memory_released();
        assert_eq!(
            netdata_promql_query_release(query.get().cast_mut()),
            types::OK
        );
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        accounting.snapshot().assert_memory_released();
    }
}
