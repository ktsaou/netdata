// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[test]
fn selectors_own_source_and_match_with_checked_borrowed_labels_after_engine_release() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut source = br#"{zone=~"x|y",missing!="present"}"#.to_vec();
        let mut selector = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_selector_compile(
                engine,
                types::Bytes::of(&source),
                ptr::null(),
                &mut selector,
                &mut report
            ),
            types::OK
        );
        source.fill(b'X');
        drop(source);
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        let mut count = 0;
        assert_eq!(
            netdata_promql_selector_matcher_count(selector, &mut count),
            types::OK
        );
        assert_eq!(count, 2);
        let mut matcher = types::Matcher::default();
        assert_eq!(
            netdata_promql_selector_matcher(selector, count, &mut matcher),
            types::INVALID_ARGUMENT
        );
        let mut nonempty = 0;
        assert_eq!(
            netdata_promql_selector_nonempty(selector, ptr::null(), &mut nonempty),
            types::OK
        );
        assert_eq!(nonempty, 1);
        for (value, expected) in [(b"x".as_slice(), 1), (b"z".as_slice(), 0)] {
            let labels = [types::Label {
                name: types::Bytes::of(b"zone"),
                value: types::Bytes::of(value),
            }];
            let mut matched = 9;
            assert_eq!(
                netdata_promql_selector_matches(
                    selector,
                    types::Labels {
                        data: labels.as_ptr(),
                        len: 1
                    },
                    ptr::null(),
                    &mut matched
                ),
                types::OK
            );
            assert_eq!(matched, expected);
        }
        let duplicate = [types::Label {
            name: types::Bytes::of(b"zone"),
            value: types::Bytes::of(b"z"),
        }; 2];
        let mut matched = 9;
        assert_eq!(
            netdata_promql_selector_matches(
                selector,
                types::Labels {
                    data: duplicate.as_ptr(),
                    len: 2
                },
                ptr::null(),
                &mut matched
            ),
            types::INVALID_ARGUMENT
        );
        assert_eq!(matched, 0);
        assert_eq!(netdata_promql_selector_release(selector), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn cancelled_protocol_operations_release_reports_and_selector_owners() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut token = ptr::null_mut();
        assert_eq!(
            netdata_promql_control_create(engine, ptr::null(), &mut token),
            types::OK
        );
        assert_eq!(netdata_promql_control_cancel(token), types::OK);
        let mut report = ptr::null_mut();
        let mut time = types::Timestamp::default();
        assert_eq!(
            netdata_promql_api_parse_time(
                engine,
                types::Bytes::of(b"NaN"),
                token,
                &mut time,
                &mut report
            ),
            types::CANCELLED
        );
        assert!(!report.is_null());
        assert_eq!(netdata_promql_report_release(report), types::OK);
        let mut duration = 17;
        assert_eq!(
            netdata_promql_api_parse_duration(
                engine,
                types::Bytes::of(b"1ms"),
                token,
                &mut duration,
                &mut report
            ),
            types::CANCELLED
        );
        assert_eq!(duration, 0);
        assert_eq!(netdata_promql_report_release(report), types::OK);
        let mut selector = ptr::null_mut();
        assert_eq!(
            netdata_promql_selector_compile(
                engine,
                types::Bytes::of(b"{}"),
                token,
                &mut selector,
                &mut report
            ),
            types::CANCELLED
        );
        assert!(selector.is_null());
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_control_release(token), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn already_cancelled_label_matching_observes_descriptors_before_conversion_allocation() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut selector = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_selector_compile(
                engine,
                types::Bytes::of(b"{}"),
                ptr::null(),
                &mut selector,
                &mut report
            ),
            types::OK
        );
        assert_eq!(netdata_promql_report_release(report), types::OK);
        let mut token = ptr::null_mut();
        assert_eq!(
            netdata_promql_control_create(engine, ptr::null(), &mut token),
            types::OK
        );
        assert_eq!(netdata_promql_control_cancel(token), types::OK);
        let labels = vec![
            types::Label {
                name: types::Bytes::of(b"zone"),
                value: types::Bytes::of(b"value")
            };
            100_000
        ];
        let before = account.snapshot();
        let mut matched = 9;
        assert_eq!(
            netdata_promql_selector_matches(
                selector,
                types::Labels {
                    data: labels.as_ptr(),
                    len: labels.len() as u64
                },
                token,
                &mut matched
            ),
            types::CANCELLED
        );
        assert_eq!(matched, 0);
        let after = account.snapshot();
        assert_eq!(
            after.memory[MemoryKind::Workspace as usize].acquired,
            before.memory[MemoryKind::Workspace as usize].acquired
        );
        assert!(
            after.memory[MemoryKind::BorrowedView as usize].peak
                >= size_of_val(labels.as_slice()) as u64
        );
        assert_eq!(after.memory[MemoryKind::BorrowedView as usize].current, 0);
        assert_eq!(netdata_promql_selector_release(selector), types::OK);
        assert_eq!(netdata_promql_control_release(token), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn protocol_errors_are_owned_and_sample_and_scalar_text_are_distinct() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let mut source = b"bad".to_vec();
        let mut time = types::Timestamp::default();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_api_parse_time(
                engine,
                types::Bytes::of(&source),
                ptr::null(),
                &mut time,
                &mut report
            ),
            types::PARSE_ERROR
        );
        source.fill(0);
        drop(source);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        let mut error = types::Error::default();
        assert_eq!(
            netdata_promql_report_error(report, 0, &mut error),
            types::OK
        );
        assert_eq!(
            checked::bytes(error.message).unwrap(),
            br#"cannot parse "bad" to a valid timestamp"#
        );
        assert_eq!(error.position_present, 0);
        assert_eq!(netdata_promql_report_release(report), types::OK);
        account.snapshot().assert_memory_released();
        let mut text = types::NumericText::default();
        assert_eq!(netdata_promql_api_timestamp_text(100, &mut text), types::OK);
        assert_eq!(&text.bytes[..text.len as usize], b"0.100");
        assert_eq!(
            netdata_promql_api_scalar_timestamp_text(100, &mut text),
            types::OK
        );
        assert_eq!(&text.bytes[..text.len as usize], b"0.1");
        assert_eq!(
            netdata_promql_api_scalar_float_text(0x8000_0000_0000_0001, &mut text),
            types::OK
        );
        assert_eq!(text.len, 327);
        assert_eq!(&text.bytes[..3], b"-0.");
        assert_eq!(text.bytes[326], b'5');
    }
}
