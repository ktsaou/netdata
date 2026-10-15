// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

unsafe fn observe_invalidation(
    context: *mut c_void,
    account: *const types::AccountingContext,
    kind: u8,
) {
    let account = unsafe { checked::accounting(account) }.unwrap();
    unsafe { fixture(context) }
        .borrowed_before_mutation
        .borrow_mut()
        .push((
            kind,
            account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
        ));
}
unsafe extern "C" fn observed_page_next(
    context: *mut c_void,
    handle: *mut c_void,
    control: *const types::CallbackControl,
    out: *mut types::Page,
    resources: *mut types::Resources,
    error: *mut types::Bytes,
) -> u32 {
    let account = unsafe { checked::callback(control) }.unwrap().account;
    unsafe { observe_invalidation(context, checked::accounting_context(account), 1) };
    unsafe { cursor_next(context, handle, control, out, resources, error) }
}
unsafe extern "C" fn observed_page_seek(
    context: *mut c_void,
    handle: *mut c_void,
    target: i64,
    control: *const types::CallbackControl,
    error: *mut types::Bytes,
) -> u32 {
    let account = unsafe { checked::callback(control) }.unwrap().account;
    unsafe { observe_invalidation(context, checked::accounting_context(account), 2) };
    unsafe { cursor_seek(context, handle, target, control, error) }
}
unsafe extern "C" fn observed_page_close(
    context: *mut c_void,
    handle: *mut c_void,
    account: *const types::AccountingContext,
) {
    unsafe { observe_invalidation(context, account, 3) };
    unsafe { cursor_close(context, handle, account) };
}

#[test]
fn foreign_page_borrows_end_before_mutation_end_failure_and_unwind_close() {
    use crate::storage::{Provider, QueryProvider, SampleCursor, Series, SeriesSelection};
    for mode in 0..3 {
        let accounting = crate::accounting::EngineAccounting::default();
        let account = accounting.query();
        let control = CoreControl::default();
        let mut fixture = Fixture::new(1);
        let mut table = fixture.table();
        table.cursor_next = Some(observed_page_next);
        table.cursor_seek = Some(observed_page_seek);
        table.cursor_close = Some(observed_page_close);
        let provider = unsafe { provider::Provider::new(&table, &account) }.unwrap();
        let bounds = crate::storage::TimeBounds {
            start_ms: 0,
            end_ms: 1000,
        };
        let query = provider.open(bounds, &control).unwrap();
        let mut selection = query
            .select(
                crate::storage::SelectionRequest {
                    matchers: &[],
                    bounds,
                    hints: crate::storage::SelectionHints::default(),
                    accounting: Some(&account),
                },
                &control,
            )
            .unwrap();
        let series = selection.next(&control).unwrap().unwrap();
        drop(selection);
        if mode == 2 {
            let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut cursor = series.cursor(bounds, &control).unwrap();
                let page = cursor
                    .next(&control, &mut crate::storage::ProviderResources::default())
                    .unwrap()
                    .unwrap();
                assert_eq!(page.samples.sample(0).time_ms, 0);
                panic!("owned cursor unwind fixture");
            }));
            assert!(unwind.is_err());
        } else {
            let mut cursor = series.cursor(bounds, &control).unwrap();
            let mut resources = crate::storage::ProviderResources::default();
            assert_eq!(
                cursor
                    .next(&control, &mut resources)
                    .unwrap()
                    .unwrap()
                    .samples
                    .sample(0)
                    .time_ms,
                0
            );
            if mode == 1 {
                // Mutate through the foreign context token without invalidating its pointer provenance.
                unsafe { ptr::addr_of_mut!((*table.context.cast::<Fixture>()).failure).write(6) };
                assert!(cursor.next(&control, &mut resources).is_err());
            } else {
                cursor.seek(1000, &control).unwrap();
                assert_eq!(
                    cursor
                        .next(&control, &mut resources)
                        .unwrap()
                        .unwrap()
                        .samples
                        .sample(0)
                        .time_ms,
                    1000
                );
                assert!(cursor.next(&control, &mut resources).unwrap().is_none());
            }
        }
        assert_eq!(
            account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
            0
        );
        drop(series);
        drop(query);
        assert_eq!(
            fixture.borrowed_before_mutation.borrow().last(),
            Some(&(3, 0))
        );
        assert!(
            fixture
                .borrowed_before_mutation
                .borrow()
                .iter()
                .all(|(_, bytes)| *bytes == 0)
        );
        for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
            assert_eq!(opened.get(), closed.get());
        }
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn adapted_pages_retain_no_owned_sample_descriptors() {
    use crate::storage::{Provider, QueryProvider, SampleCursor, Series, SeriesSelection};
    let mut retained_pages = 0;
    for page_size in [1, 17, 1024] {
        let accounting = crate::accounting::EngineAccounting::default();
        let account = accounting.query();
        let control = CoreControl::default();
        let mut fixture = Fixture::new(page_size);
        fixture.points = (0..page_size)
            .map(|index| types::Sample {
                time_ms: index as i64 * 1000,
                start_time_ms: 0,
                value: types::Value {
                    kind: types::VALUE_FLOAT,
                    reserved: 0,
                    payload: types::ValuePayload {
                        float_bits: (index as f64).to_bits(),
                    },
                },
            })
            .collect();
        let table = fixture.table();
        let provider = unsafe { provider::Provider::new(&table, &account) }.unwrap();
        let bounds = crate::storage::TimeBounds {
            start_ms: 0,
            end_ms: page_size as i64 * 1000,
        };
        let query = provider.open(bounds, &control).unwrap();
        let mut selection = query
            .select(
                crate::storage::SelectionRequest {
                    matchers: &[],
                    bounds,
                    hints: crate::storage::SelectionHints::default(),
                    accounting: Some(&account),
                },
                &control,
            )
            .unwrap();
        let series = selection.next(&control).unwrap().unwrap();
        drop(selection);
        let mut cursor = series.cursor(bounds, &control).unwrap();
        let before = account.snapshot().memory[MemoryKind::Workspace as usize].current;
        let mut resources = crate::storage::ProviderResources::default();
        {
            let page = cursor.next(&control, &mut resources).unwrap().unwrap();
            assert_eq!(page.samples.len(), page_size);
            for index in (0..page_size).rev() {
                let sample = page.samples.sample(index);
                assert_eq!(sample.time_ms, index as i64 * 1000);
                let crate::value::SampleValueRef::Float(value) = sample.value else {
                    panic!("float page");
                };
                assert_eq!(value.to_bits(), (index as f64).to_bits());
            }
            let observed = account.snapshot();
            let retained = observed.memory[MemoryKind::Workspace as usize].current - before;
            println!(
                "{{\"adapter_page\":{page_size},\"retained_workspace\":{retained},\"borrowed_bytes\":{},\"fetched\":{}}}",
                observed.memory[MemoryKind::BorrowedView as usize].current,
                observed.samples_fetched
            );
            retained_pages += usize::from(retained != 0);
            assert_eq!(observed.samples_fetched, page_size as u64);
            assert_eq!(
                observed.memory[MemoryKind::BorrowedView as usize].current,
                (page_size * size_of::<types::Sample>()) as u64
            );
        }
        drop(cursor);
        drop(series);
        drop(query);
        for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
            assert_eq!(opened.get(), closed.get());
        }
        account.snapshot().assert_memory_released();
    }
    assert_eq!(
        retained_pages, 0,
        "C pages must not retain a second sample array"
    );
}

#[test]
fn source_histogram_formatter_errors_survive_foreign_and_plan_release() {
    let spans = [types::Span {
        offset: 0,
        length: 1,
    }];
    let buckets = [1.0f64];
    for (schema, expected) in [
        (
            9,
            &b"unexpected error: runtime error: index out of range [9] with length 9"[..],
        ),
        (
            i32::MIN,
            &b"unexpected error: runtime error: negative shift amount"[..],
        ),
    ] {
        for page_size in [1, 17, 1024] {
            for expression in [
                &b"count_values(\"v\",m)"[..],
                &b"histogram_stdvar(m)"[..],
                &b"histogram_quantile(.5,m)"[..],
                &b"histogram_fraction(0,1,m)"[..],
                &b"m </ 1"[..],
            ] {
                unsafe {
                    let engine = engine();
                    let accounting = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
                    let query = compiled(engine, expression);
                    let histogram = types::FloatHistogram {
                        schema,
                        count_bits: 1.0f64.to_bits(),
                        sum_bits: 1.0f64.to_bits(),
                        positive_spans: types::SpanArray {
                            data: spans.as_ptr(),
                            len: 1,
                            present: 1,
                            reserved: 0,
                        },
                        positive_buckets: types::F64Array {
                            data: buckets.as_ptr(),
                            len: 1,
                            present: 1,
                            reserved: 0,
                        },
                        ..types::FloatHistogram::default()
                    };
                    let mut fixture = Fixture::new(page_size);
                    fixture.points = vec![types::Sample {
                        time_ms: 0,
                        start_time_ms: 0,
                        value: types::Value {
                            kind: types::VALUE_FLOAT_HISTOGRAM,
                            reserved: 0,
                            payload: types::ValuePayload {
                                float_histogram: &histogram,
                            },
                        },
                    }];
                    let table = fixture.table();
                    let mut result = ptr::null_mut();
                    let mut report = ptr::null_mut();
                    assert_eq!(
                        netdata_promql_collect(
                            query,
                            &table,
                            &instant(0),
                            ptr::null(),
                            &mut result,
                            &mut report,
                        ),
                        types::EVALUATION_ERROR
                    );
                    assert!(result.is_null());
                    assert!(!report.is_null());
                    for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                        assert!(opened.get() > 0);
                        assert_eq!(opened.get(), closed.get());
                    }
                    assert_eq!(netdata_promql_query_release(query), types::OK);
                    assert_eq!(netdata_promql_engine_release(engine), types::OK);
                    drop(fixture);
                    let mut error = types::Error::default();
                    assert_eq!(
                        netdata_promql_report_error(report, 0, &mut error),
                        types::OK
                    );
                    assert_eq!((error.kind, error.position_present), (6, 0));
                    assert_eq!(checked::bytes(error.message).unwrap(), expected);
                    assert_eq!(netdata_promql_report_release(report), types::OK);
                    accounting.snapshot().assert_memory_released();
                }
            }
        }
    }
}

#[test]
fn nested_foreign_views_preserve_bits_and_union_the_actual_exposed_regions() {
    use crate::storage::{Provider, QueryProvider, SampleCursor, Series, SeriesSelection};
    let accounting = crate::accounting::EngineAccounting::default();
    let account = accounting.query();
    let control = CoreControl::default();
    let spans = [
        types::Span {
            offset: -1,
            length: 2,
        },
        types::Span {
            offset: 3,
            length: 1,
        },
    ];
    let floats = [-0.0, f64::from_bits(0x7ff8_0000_0000_0123), 4.0, 6.0];
    let integers = [9_007_199_254_740_993i64, -3, 7, 9];
    let span_array = types::SpanArray {
        data: spans.as_ptr(),
        len: 2,
        present: 1,
        reserved: 0,
    };
    let float_histogram = Box::new(types::FloatHistogram {
        reset_hint: 255,
        schema: -53,
        zero_threshold_bits: (-0.0f64).to_bits(),
        zero_count_bits: floats[1].to_bits(),
        count_bits: (-1.0f64).to_bits(),
        sum_bits: 0xfff8_0000_0000_0456,
        positive_spans: span_array,
        negative_spans: span_array,
        positive_buckets: types::F64Array {
            data: floats.as_ptr(),
            len: 3,
            present: 1,
            reserved: 0,
        },
        negative_buckets: types::F64Array {
            data: unsafe { floats.as_ptr().add(1) },
            len: 3,
            present: 1,
            reserved: 0,
        },
        custom_values: types::F64Array {
            data: ptr::null(),
            len: 0,
            present: 1,
            reserved: 0,
        },
    });
    let integer_histogram = Box::new(types::IntegerHistogram {
        reset_hint: 2,
        schema: 0,
        zero_threshold_bits: 0,
        zero_count: 9_007_199_254_740_993,
        count: u64::MAX,
        sum_bits: 0x7ff0_0000_0000_0002,
        positive_spans: span_array,
        negative_spans: types::SpanArray::default(),
        positive_buckets: types::I64Array {
            data: integers.as_ptr(),
            len: 3,
            present: 1,
            reserved: 0,
        },
        negative_buckets: types::I64Array {
            data: unsafe { integers.as_ptr().add(1) },
            len: 3,
            present: 1,
            reserved: 0,
        },
        custom_values: types::F64Array {
            data: floats.as_ptr(),
            len: 4,
            present: 1,
            reserved: 0,
        },
    });
    let mut fixture = Fixture::new(1024);
    fixture.points = vec![
        types::Sample {
            time_ms: 0,
            start_time_ms: 0,
            value: types::Value {
                kind: types::VALUE_FLOAT,
                reserved: 0,
                payload: types::ValuePayload {
                    float_bits: (-0.0f64).to_bits(),
                },
            },
        },
        types::Sample {
            time_ms: 1000,
            start_time_ms: 17,
            value: types::Value {
                kind: types::VALUE_FLOAT_HISTOGRAM,
                reserved: 0,
                payload: types::ValuePayload {
                    float_histogram: &*float_histogram,
                },
            },
        },
        types::Sample {
            time_ms: 2000,
            start_time_ms: 19,
            value: types::Value {
                kind: types::VALUE_FLOAT_HISTOGRAM,
                reserved: 0,
                payload: types::ValuePayload {
                    float_histogram: &*float_histogram,
                },
            },
        },
        types::Sample {
            time_ms: 3000,
            start_time_ms: 23,
            value: types::Value {
                kind: types::VALUE_INTEGER_HISTOGRAM,
                reserved: 0,
                payload: types::ValuePayload {
                    integer_histogram: &*integer_histogram,
                },
            },
        },
    ];
    let table = fixture.table();
    let provider = unsafe { provider::Provider::new(&table, &account) }.unwrap();
    let bounds = crate::storage::TimeBounds {
        start_ms: 0,
        end_ms: 3000,
    };
    let query = provider.open(bounds, &control).unwrap();
    let mut selection = query
        .select(
            crate::storage::SelectionRequest {
                matchers: &[],
                bounds,
                hints: crate::storage::SelectionHints::default(),
                accounting: Some(&account),
            },
            &control,
        )
        .unwrap();
    let series = selection.next(&control).unwrap().unwrap();
    drop(selection);
    let mut cursor = series.cursor(bounds, &control).unwrap();
    let mut resources = crate::storage::ProviderResources::default();
    {
        let page = cursor.next(&control, &mut resources).unwrap().unwrap();
        assert_eq!(page.samples.len(), 4);
        let crate::value::SampleValueRef::Float(value) = page.samples.sample(0).value else {
            panic!("float tag");
        };
        assert_eq!(value.to_bits(), (-0.0f64).to_bits());
        let sample = page.samples.sample(1);
        assert_eq!(sample.start_time_ms, 17);
        let crate::value::SampleValueRef::FloatHistogram(value) = sample.value else {
            panic!("float histogram tag");
        };
        assert_eq!((value.reset_hint, value.schema), (255, -53));
        assert_eq!(value.zero_count.to_bits(), floats[1].to_bits());
        assert_eq!(value.sum.to_bits(), float_histogram.sum_bits);
        assert_eq!(value.custom_values, Some(&[][..]));
        assert_eq!(
            value.positive_buckets.unwrap()[1].to_bits(),
            floats[1].to_bits()
        );
        let crate::value::SampleValueRef::IntegerHistogram(value) = page.samples.sample(3).value
        else {
            panic!("integer histogram tag");
        };
        assert_eq!(
            (value.count, value.zero_count),
            (u64::MAX, 9_007_199_254_740_993)
        );
        assert_eq!(value.positive_buckets.unwrap()[0], 9_007_199_254_740_993);
        assert_eq!(value.negative_spans, None);
        assert_eq!(value.sum.to_bits(), 0x7ff0_0000_0000_0002);
        std::thread::scope(|scope| {
            let samples = page.samples;
            scope
                .spawn(move || {
                    let crate::value::SampleValueRef::IntegerHistogram(integer) =
                        samples.sample(3).value
                    else {
                        panic!("integer histogram tag");
                    };
                    let crate::value::SampleValueRef::FloatHistogram(float) =
                        samples.sample(1).value
                    else {
                        panic!("float histogram tag");
                    };
                    assert_eq!(integer.count, u64::MAX);
                    assert_eq!(float.sum.to_bits(), 0xfff8_0000_0000_0456);
                    assert_eq!(float.custom_values, Some(&[][..]));
                    assert!(format!("{samples:?}").contains("start_time_ms: 17"));
                })
                .join()
                .unwrap();
        });
        let expected = 4 * size_of::<types::Sample>()
            + size_of::<types::FloatHistogram>()
            + size_of::<types::IntegerHistogram>()
            + size_of_val(&spans)
            + size_of_val(&floats)
            + size_of_val(&integers);
        assert_ne!(
            size_of::<crate::value::SampleRef<'_>>(),
            size_of::<types::Sample>()
        );
        let borrowed = account.snapshot().memory[MemoryKind::BorrowedView as usize];
        assert_eq!(borrowed.current, expected as u64);
        assert_eq!(borrowed.peak, expected as u64);
    }
    assert!(cursor.next(&control, &mut resources).unwrap().is_none());
    assert_eq!(
        account.snapshot().memory[MemoryKind::BorrowedView as usize].current,
        0
    );
    cursor.seek(1000, &control).unwrap();
    assert_eq!(
        cursor
            .next(&control, &mut resources)
            .unwrap()
            .unwrap()
            .samples
            .len(),
        3
    );
    drop(cursor);
    drop(series);
    drop(query);
    for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
        assert_eq!(opened.get(), closed.get());
    }
    account.snapshot().assert_memory_released();
    drop(account);
    accounting.snapshot().assert_memory_released();
}

#[test]
fn foreign_page_shapes_precede_cancellation_and_unread_tail_timestamps_stay_in_the_walker() {
    use crate::storage::{
        Provider, QueryProvider, SampleCursor, Series, SeriesSelection, StorageError,
    };
    for mode in 0..5 {
        for cancelled in [false, true] {
            if mode >= 3 && cancelled {
                continue;
            }
            let accounting = crate::accounting::EngineAccounting::default();
            let account = accounting.query();
            let control = CoreControl::default();
            let histogram = types::FloatHistogram {
                positive_buckets: types::F64Array {
                    present: 2,
                    ..types::F64Array::default()
                },
                ..types::FloatHistogram::default()
            };
            let mut fixture = Fixture::new(17);
            fixture.points.truncate(3);
            let expected = match mode {
                0 => {
                    fixture.points[2].value.reserved = 1;
                    "nonzero reserved value field"
                }
                1 => {
                    fixture.points[2].value.kind = 99;
                    "invalid sample value tag"
                }
                2 => {
                    fixture.points[2].value.kind = types::VALUE_FLOAT_HISTOGRAM;
                    fixture.points[2].value.payload.float_histogram = &histogram;
                    "invalid presence bit"
                }
                3 => {
                    fixture.points[2].time_ms = 1000;
                    "first consumer failure"
                }
                _ => {
                    fixture.points[2].time_ms = 1000;
                    "sample timestamps are not strictly increasing"
                }
            };
            let table = fixture.table();
            let provider = unsafe { provider::Provider::new(&table, &account) }.unwrap();
            let bounds = crate::storage::TimeBounds {
                start_ms: 0,
                end_ms: 3000,
            };
            let query = provider.open(bounds, &control).unwrap();
            let mut selection = query
                .select(
                    crate::storage::SelectionRequest {
                        matchers: &[],
                        bounds,
                        hints: crate::storage::SelectionHints::default(),
                        accounting: Some(&account),
                    },
                    &control,
                )
                .unwrap();
            let series = selection.next(&control).unwrap().unwrap();
            drop(selection);
            let error = if mode < 3 {
                let mut cursor = series.cursor(bounds, &control).unwrap();
                if cancelled {
                    control.cancellation.cancel();
                }
                cursor
                    .next(&control, &mut crate::storage::ProviderResources::default())
                    .unwrap_err()
            } else {
                let mut visits = 0;
                let error = crate::storage::scan_until(&series, bounds, &control, &account, |_| {
                    visits += 1;
                    if mode == 3 {
                        Err(StorageError::Invalid("first consumer failure"))
                    } else {
                        Ok(std::ops::ControlFlow::Break(()))
                    }
                })
                .unwrap_err();
                assert_eq!(visits, 1);
                error
            };
            assert!(matches!(error, StorageError::Invalid(message) if message == expected));
            let observed = account.snapshot();
            assert_eq!((observed.pages, observed.samples_fetched), (1, 3));
            drop(series);
            drop(query);
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            account.snapshot().assert_memory_released();
        }
    }
}

#[test]
fn returned_c_pages_are_counted_before_conversion_cancellation() {
    unsafe {
        let engine = engine();
        let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
        let query = compiled(engine, b"m[5s]");
        let mut token = ptr::null_mut();
        assert_eq!(
            netdata_promql_control_create(engine, ptr::null(), &mut token),
            types::OK
        );
        let mut fixture = Fixture::new(7);
        fixture.cancel_stage = 6;
        fixture.cancellation = token;
        let table = fixture.table();
        let mut result = ptr::null_mut();
        let mut report = ptr::null_mut();
        assert_eq!(
            netdata_promql_collect(
                query,
                &table,
                &instant(10_000),
                token,
                &mut result,
                &mut report
            ),
            types::CANCELLED
        );
        assert_eq!(fixture.cancelled_status.get(), types::OK);
        assert!(result.is_null());
        let mut observed = types::QueryObservations::default();
        assert_eq!(
            netdata_promql_report_observations(report, &mut observed),
            types::OK
        );
        assert_eq!((observed.pages, observed.samples_fetched), (1, 5));
        assert_eq!(
            observed.memory[MemoryKind::BorrowedView as usize].peak,
            (5 * size_of::<types::Sample>()) as u64
        );
        assert_eq!(
            observed.memory[MemoryKind::BorrowedView as usize].current,
            0
        );
        assert_eq!(observed.cancellation_references.external.peak, 0);
        assert!(observed.cancellation_references.engine_owned.peak > 0);
        for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
            assert_eq!(opened.get(), closed.get());
        }
        assert_eq!(netdata_promql_report_release(report), types::OK);
        assert_eq!(netdata_promql_query_release(query), types::OK);
        assert_eq!(netdata_promql_control_release(token), types::OK);
        assert_eq!(netdata_promql_engine_release(engine), types::OK);
        account.snapshot().assert_memory_released();
    }
}

#[test]
fn successful_opens_close_after_invalid_nested_metadata_or_zero_page_size() {
    for bad_labels in [false, true] {
        unsafe {
            let engine = engine();
            let account = borrow::<Engine, _>(engine).unwrap().core.accounting.clone();
            let query = compiled(engine, b"m");
            let mut fixture = Fixture::new(if bad_labels { 1 } else { 0 });
            if bad_labels {
                fixture.labels[1].value = types::Bytes {
                    data: ptr::null(),
                    len: 1,
                };
            }
            let table = fixture.table();
            let mut result = ptr::null_mut();
            let mut report = ptr::null_mut();
            assert_ne!(
                netdata_promql_collect(
                    query,
                    &table,
                    &instant(1000),
                    ptr::null(),
                    &mut result,
                    &mut report
                ),
                types::OK
            );
            assert!(result.is_null());
            assert_eq!(fixture.opened[2].get(), 1);
            assert_eq!(fixture.opened[3].get(), usize::from(!bad_labels));
            let mut observed = types::QueryObservations::default();
            assert_eq!(
                netdata_promql_report_observations(report, &mut observed),
                types::OK
            );
            assert_eq!((observed.pages, observed.samples_fetched), (0, 0));
            for (opened, closed) in fixture.opened.iter().zip(&fixture.closed) {
                assert_eq!(opened.get(), closed.get());
            }
            assert_eq!(netdata_promql_report_release(report), types::OK);
            assert_eq!(netdata_promql_query_release(query), types::OK);
            assert_eq!(netdata_promql_engine_release(engine), types::OK);
            account.snapshot().assert_memory_released();
        }
    }
}
