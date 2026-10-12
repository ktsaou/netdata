// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn precise(time: types::Timestamp, lookback_ns: i64) -> types::PreciseRequest {
    types::PreciseRequest {
        header: types::Header::of::<types::PreciseRequest>(),
        kind: types::REQUEST_INSTANT,
        start: time,
        lookback_ns,
        ..types::PreciseRequest::default()
    }
}

#[test]
fn precise_c_requests_preserve_original_clock_and_positive_sub_ms_lookback() {
    for page_size in [1, 17, 1024] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let mut fixture = Fixture::new(page_size);
            let provider = fixture.table();
            for (source, request, expected_rows) in [
                (
                    b"vector(time())".as_slice(),
                    precise(
                        types::Timestamp {
                            unix_seconds: -1,
                            nanoseconds: 999_999_999,
                            reserved: 0,
                        },
                        300_000_000_000,
                    ),
                    1,
                ),
                (
                    b"m".as_slice(),
                    precise(
                        types::Timestamp {
                            unix_seconds: 2,
                            ..types::Timestamp::default()
                        },
                        500_000,
                    ),
                    0,
                ),
            ] {
                let query = compiled(engine, source);
                let mut result = ptr::null_mut();
                let mut report = ptr::null_mut();
                assert_eq!(
                    netdata_promql_collect_precise(
                        query,
                        &provider,
                        &request,
                        ptr::null(),
                        &mut result,
                        &mut report
                    ),
                    types::OK
                );
                let mut info = types::ResultInfo::default();
                assert_eq!(netdata_promql_result_info(result, &mut info), types::OK);
                assert_eq!(info.series_count, expected_rows);
                if expected_rows == 1 {
                    let mut point = types::PointInfo::default();
                    assert_eq!(
                        netdata_promql_result_point(result, 0, 0, &mut point),
                        types::OK
                    );
                    assert_eq!((point.time_ms, point.value_bits), (0, 0));
                }
                assert_eq!(netdata_promql_result_release(result), types::OK);
                assert_eq!(netdata_promql_report_release(report), types::OK);
                assert_eq!(netdata_promql_query_release(query), types::OK);
            }
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
        }
    }
}

#[test]
fn precise_c_sub_ms_step_reaches_source_runtime_fault_and_releases_provider() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m");
        let mut fixture = Fixture::new(1);
        let provider = fixture.table();
        let request = types::PreciseRequest {
            kind: types::REQUEST_RANGE,
            end: types::Timestamp {
                nanoseconds: 2_000_000,
                ..types::Timestamp::default()
            },
            interval_ns: 500_000,
            ..precise(types::Timestamp::default(), 300_000_000_000)
        };
        let mut result = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect_precise(
                query,
                &provider,
                &request,
                ptr::null(),
                &mut result,
                &mut report
            ),
            types::EVALUATION_ERROR
        );
        assert!(result.is_null() && !report.is_null());
        let mut error = types::Error::default();
        assert_eq!(
            netdata_promql_report_error(report, 0, &mut error),
            types::OK
        );
        assert_eq!(
            checked::bytes(error.message).unwrap(),
            b"unexpected error: runtime error: integer divide by zero"
        );
        assert_eq!(fixture.opened[0].get(), 1);
        assert_eq!(fixture.opened[1].get(), 1);
        for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
            assert_eq!(opened.get(), closed.get());
        }
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn malformed_precise_request_rejects_before_provider_open_and_full_prefix_access() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m");
        let mut fixture = Fixture::new(1);
        let provider = fixture.table();
        for request in [
            types::PreciseRequest {
                header: types::Header::of::<types::Header>(),
                ..precise(types::Timestamp::default(), 1)
            },
            precise(
                types::Timestamp {
                    nanoseconds: 1_000_000_000,
                    ..types::Timestamp::default()
                },
                1,
            ),
            precise(
                types::Timestamp {
                    reserved: 1,
                    ..types::Timestamp::default()
                },
                1,
            ),
            types::PreciseRequest {
                reserved: 1,
                ..precise(types::Timestamp::default(), 1)
            },
        ] {
            let mut result = ptr::null_mut();
            let mut report = ptr::null_mut();
            assert_eq!(
                netdata_promql_collect_precise(
                    query,
                    &provider,
                    &request,
                    ptr::null(),
                    &mut result,
                    &mut report
                ),
                types::INVALID_ARGUMENT
            );
            assert!(result.is_null() && report.is_null());
        }
        assert_eq!(fixture.opened[0].get(), 0);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}
