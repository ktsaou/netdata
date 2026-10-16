// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::{
    accounting::{EngineAccounting, MemoryObservation},
    histogram::{FloatHistogram, IntegerHistogram, Span},
    result::{OutputChunk, OutputError, OutputSink},
    storage::{Control, LabelRef},
    value::{SampleRef, SampleValueRef},
};

struct Points<'a> {
    labels: &'a [LabelRef<'a>],
    points: &'a [SampleRef<'a>],
    verify: bool,
    calls: Cell<u64>,
    aborts: Cell<u64>,
    checksum: Cell<u64>,
}
impl Points<'_> {
    fn table(&mut self) -> types::Output {
        types::Output {
            header: types::Header::of::<types::Output>(),
            context: (self as *mut Self).cast(),
            begin: Some(begin),
            chunk: Some(chunk),
            commit: Some(commit),
            abort: Some(abort),
        }
    }
}
unsafe extern "C" fn begin(
    _: *mut c_void,
    _: *const types::OutputHeader,
    _: *const types::CallbackControl,
    _: *mut types::Bytes,
) -> u32 {
    types::OK
}
unsafe extern "C" fn commit(
    _: *mut c_void,
    _: *const types::QueryObservations,
    _: *const types::CallbackControl,
    _: *mut types::Bytes,
) -> u32 {
    types::OK
}
unsafe extern "C" fn abort(context: *mut c_void, _: *const types::AccountingContext) {
    let points = unsafe { &*context.cast::<Points<'_>>() };
    points.aborts.set(points.aborts.get() + 1);
}

unsafe fn floats(actual: types::F64Array, expected: Option<&[f64]>) {
    assert_eq!(actual.present, u32::from(expected.is_some()));
    assert_eq!(actual.reserved, 0);
    assert_eq!(actual.len as usize, expected.map_or(0, <[f64]>::len));
    if let Some(expected) = expected {
        for (actual, expected) in
            unsafe { std::slice::from_raw_parts(actual.data, actual.len as usize) }
                .iter()
                .zip(expected)
        {
            assert_eq!(actual.to_bits(), expected.to_bits());
        }
    } else {
        assert!(actual.data.is_null());
    }
}
unsafe fn spans(actual: types::SpanArray, expected: Option<&[Span]>) {
    assert_eq!(actual.present, u32::from(expected.is_some()));
    assert_eq!(actual.reserved, 0);
    assert_eq!(actual.len as usize, expected.map_or(0, <[Span]>::len));
    if let Some(expected) = expected {
        assert_eq!(
            unsafe { std::slice::from_raw_parts(actual.data, actual.len as usize) },
            expected
        );
    } else {
        assert!(actual.data.is_null());
    }
}
unsafe fn point(actual: &types::Sample, expected: SampleRef<'_>) {
    assert_eq!(actual.time_ms, expected.time_ms);
    assert_eq!(actual.start_time_ms, expected.start_time_ms);
    assert_eq!(actual.value.reserved, 0);
    match expected.value {
        SampleValueRef::Float(value) => {
            assert_eq!(actual.value.kind, types::VALUE_FLOAT);
            assert_eq!(unsafe { actual.value.payload.float_bits }, value.to_bits());
        }
        SampleValueRef::FloatHistogram(value) => {
            assert_eq!(actual.value.kind, types::VALUE_FLOAT_HISTOGRAM);
            let actual = unsafe { &*actual.value.payload.float_histogram };
            assert_eq!(
                (
                    actual.reset_hint,
                    actual.schema,
                    actual.zero_threshold_bits,
                    actual.zero_count_bits,
                    actual.count_bits,
                    actual.sum_bits
                ),
                (
                    value.reset_hint as u32,
                    value.schema,
                    value.zero_threshold.to_bits(),
                    value.zero_count.to_bits(),
                    value.count.to_bits(),
                    value.sum.to_bits()
                )
            );
            unsafe {
                spans(actual.positive_spans, value.positive_spans);
                spans(actual.negative_spans, value.negative_spans);
                floats(actual.positive_buckets, value.positive_buckets);
                floats(actual.negative_buckets, value.negative_buckets);
                floats(actual.custom_values, value.custom_values);
            }
        }
        SampleValueRef::IntegerHistogram(value) => {
            assert_eq!(actual.value.kind, types::VALUE_INTEGER_HISTOGRAM);
            let actual = unsafe { &*actual.value.payload.integer_histogram };
            assert_eq!(
                (
                    actual.reset_hint,
                    actual.schema,
                    actual.zero_threshold_bits,
                    actual.zero_count,
                    actual.count,
                    actual.sum_bits
                ),
                (
                    value.reset_hint as u32,
                    value.schema,
                    value.zero_threshold.to_bits(),
                    value.zero_count,
                    value.count,
                    value.sum.to_bits()
                )
            );
            unsafe {
                spans(actual.positive_spans, value.positive_spans);
                spans(actual.negative_spans, value.negative_spans);
                floats(actual.custom_values, value.custom_values);
            }
            for (actual, expected) in [
                (actual.positive_buckets, value.positive_buckets),
                (actual.negative_buckets, value.negative_buckets),
            ] {
                assert_eq!(actual.present, u32::from(expected.is_some()));
                assert_eq!(actual.reserved, 0);
                assert_eq!(actual.len as usize, expected.map_or(0, <[i64]>::len));
                if let Some(expected) = expected {
                    assert_eq!(
                        unsafe { std::slice::from_raw_parts(actual.data, actual.len as usize) },
                        expected
                    );
                } else {
                    assert!(actual.data.is_null());
                }
            }
        }
    }
}
unsafe extern "C" fn chunk(
    context: *mut c_void,
    value: *const types::Chunk,
    control: *const types::CallbackControl,
    _: *mut types::Bytes,
) -> u32 {
    let fixture = unsafe { &*context.cast::<Points<'_>>() };
    let value = unsafe { &*value };
    assert_eq!(value.kind, types::CHUNK_SERIES);
    let series = unsafe { value.payload.series };
    assert_eq!(series.series_id, usize::MAX as u64);
    assert_eq!(series.labels.len as usize, fixture.labels.len());
    assert_eq!(series.points.len as usize, fixture.points.len());
    let points =
        unsafe { std::slice::from_raw_parts(series.points.data, series.points.len as usize) };
    if fixture.verify {
        assert_eq!(value.reserved, 0);
        let mut state = types::ControlState::default();
        assert_eq!(
            unsafe { netdata_promql_callback_control_state(control, &mut state) },
            types::OK
        );
        for (actual, expected) in
            unsafe { std::slice::from_raw_parts(series.labels.data, series.labels.len as usize) }
                .iter()
                .zip(fixture.labels)
        {
            assert_eq!(
                unsafe { checked::bytes(actual.name) }.unwrap(),
                expected.name
            );
            assert_eq!(
                unsafe { checked::bytes(actual.value) }.unwrap(),
                expected.value
            );
        }
        for (actual, expected) in points.iter().zip(fixture.points) {
            unsafe { point(actual, *expected) };
        }
    }
    let checksum = points.iter().fold(fixture.checksum.get(), |sum, point| {
        sum.wrapping_add(point.time_ms as u64)
            .wrapping_add(point.start_time_ms as u64)
            .wrapping_add(if point.value.kind == types::VALUE_FLOAT {
                unsafe { point.value.payload.float_bits }
            } else {
                point.value.kind as u64
            })
    });
    fixture.checksum.set(std::hint::black_box(checksum));
    fixture.calls.set(fixture.calls.get() + 1);
    types::OK
}

fn measured(
    labels: &[LabelRef<'_>],
    points: &[SampleRef<'_>],
    rounds: u64,
    verify: bool,
) -> (u128, MemoryObservation, u64) {
    let engine = EngineAccounting::default();
    let account = engine.query();
    let control = Control::default();
    let mut fixture = Points {
        labels,
        points,
        verify,
        calls: Cell::new(0),
        aborts: Cell::new(0),
        checksum: Cell::new(0),
    };
    let table = fixture.table();
    let mut sink = unsafe { output::Sink::new(&table) }.unwrap();
    sink.bind(&account);
    let before = account.snapshot();
    let started = std::time::Instant::now();
    for _ in 0..rounds {
        sink.chunk_checked(
            std::hint::black_box(OutputChunk::Series {
                series_id: usize::MAX,
                labels,
                points,
            }),
            &control,
            &account,
        )
        .unwrap();
    }
    let elapsed = started.elapsed().as_nanos();
    let after = account.snapshot();
    assert_eq!(fixture.calls.get(), rounds);
    let expected = points
        .iter()
        .fold(0u64, |sum, point| {
            sum.wrapping_add(point.time_ms as u64)
                .wrapping_add(point.start_time_ms as u64)
                .wrapping_add(match point.value {
                    SampleValueRef::Float(value) => value.to_bits(),
                    SampleValueRef::IntegerHistogram(_) => types::VALUE_INTEGER_HISTOGRAM as u64,
                    SampleValueRef::FloatHistogram(_) => types::VALUE_FLOAT_HISTOGRAM as u64,
                })
        })
        .wrapping_mul(rounds);
    assert_eq!(fixture.checksum.get(), expected);
    assert_eq!(
        after.work - before.work,
        (labels.len() + 2 * points.len()) as u64 * rounds
    );
    assert_eq!(
        after.memory[MemoryKind::Workspace as usize].current,
        before.memory[MemoryKind::Workspace as usize].current
    );
    let memory = after.memory[MemoryKind::Workspace as usize];
    assert_eq!(
        memory.acquired - before.memory[MemoryKind::Workspace as usize].acquired,
        memory.released - before.memory[MemoryKind::Workspace as usize].released
    );
    sink.abort();
    drop(sink);
    assert_eq!(fixture.aborts.get(), 1);
    drop(account);
    engine.snapshot().assert_memory_released();
    (elapsed, memory, expected)
}

const LABELS: [LabelRef<'_>; 2] = [
    LabelRef {
        name: b"__name__",
        value: b"m\0\xff",
    },
    LabelRef {
        name: b"zone\xff",
        value: b"",
    },
];
fn float(time_ms: i64, start_time_ms: i64, bits: u64) -> SampleRef<'static> {
    SampleRef {
        time_ms,
        start_time_ms,
        value: SampleValueRef::Float(f64::from_bits(bits)),
    }
}

#[test]
fn series_descriptors_preserve_complete_payloads_and_release_temporary_owners() {
    for bits in [
        0,
        1,
        0x8000_0000_0000_0000,
        0x3ff0_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff8_1234_5678_9abc,
        crate::value::STALE_NAN_BITS,
    ] {
        let values = [
            float(i64::MIN, i64::MAX, bits),
            float(-1, -42, bits ^ (1 << 63)),
        ];
        for count in 0..=2 {
            for labels in [&[][..], &LABELS[..]] {
                measured(labels, &values[..count], 3, true);
            }
        }
    }
    let integer = IntegerHistogram {
        reset_hint: 2,
        schema: 1,
        zero_threshold: -0.0,
        zero_count: 3,
        count: 17,
        sum: f64::from_bits(0x7ff8_1234_5678_9abc),
        positive_spans: Some(vec![Span {
            offset: -2,
            length: 2,
        }]),
        negative_spans: Some(vec![]),
        positive_buckets: Some(vec![4, -1]),
        negative_buckets: None,
        custom_values: Some(std::sync::Arc::from([1.0, 2.0])),
    };
    let float_histogram = FloatHistogram {
        reset_hint: 3,
        schema: -53,
        zero_threshold: 0.25,
        zero_count: -0.0,
        count: 7.0,
        sum: -1.0,
        positive_spans: Some(
            vec![Span {
                offset: 0,
                length: 2,
            }]
            .into(),
        ),
        negative_spans: None,
        positive_buckets: Some(vec![1.0, -0.0].into()),
        negative_buckets: Some(vec![].into()),
        custom_values: None,
    };
    let integer = SampleRef {
        time_ms: -1234,
        start_time_ms: -9000,
        value: SampleValueRef::IntegerHistogram(integer.as_ref()),
    };
    let histogram = SampleRef {
        time_ms: i64::MAX,
        start_time_ms: i64::MIN,
        value: SampleValueRef::FloatHistogram(float_histogram.as_ref()),
    };
    measured(&LABELS, &[integer], 3, true);
    measured(&[], &[histogram], 3, true);
    let mixed: [SampleRef<'_>; 15] = std::array::from_fn(|index| match index % 3 {
        0 => integer,
        1 => histogram,
        _ => float(index as i64, -1, index as u64),
    });
    measured(&LABELS, &mixed, 3, true);
}

#[test]
fn singleton_float_descriptors_do_not_acquire_temporary_heap_memory() {
    let (_, memory, _) = measured(&[], &[float(-1, 42, (-0.0f64).to_bits())], 17, true);
    assert_eq!(
        memory.acquired, 0,
        "singleton C descriptors must borrow stack storage"
    );
    assert_eq!(memory.peak, 0);
}

#[test]
fn singleton_float_delivery_preserves_work_interruption_boundaries() {
    for threshold in 1..=4 {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let control = Control::default();
        let values = [float(42, 11, 1.0f64.to_bits())];
        let mut fixture = Points {
            labels: &LABELS,
            points: &values,
            verify: true,
            calls: Cell::new(0),
            aborts: Cell::new(0),
            checksum: Cell::new(0),
        };
        let table = fixture.table();
        let mut sink = unsafe { output::Sink::new(&table) }.unwrap();
        sink.bind(&account);
        let result =
            crate::test_storage::interrupt_after_work(&account, &control, threshold, || {
                sink.chunk_checked(
                    OutputChunk::Series {
                        series_id: usize::MAX,
                        labels: &LABELS,
                        points: &values,
                    },
                    &control,
                    &account,
                )
            });
        assert!(matches!(
            result,
            Err(OutputError::Interrupted(
                crate::storage::Interrupted::Cancelled
            ))
        ));
        assert_eq!(account.snapshot().work, threshold);
        assert_eq!(fixture.calls.get(), 0);
        drop(sink);
        assert_eq!(fixture.aborts.get(), 1);
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}

#[test]
#[ignore = "explicit source-bound adapter timing; no result collection"]
fn measure_streamed_point_descriptors() {
    let pair = [float(1234, -1000, 0x3ff0_0000_0000_0000), float(-42, 7, 1)];
    let histogram = FloatHistogram::default();
    let native = [SampleRef {
        time_ms: 17,
        start_time_ms: 3,
        value: SampleValueRef::FloatHistogram(histogram.as_ref()),
    }];
    let labels: [LabelRef<'_>; 17] = std::array::from_fn(|_| LABELS[1]);
    for (name, labels, points, rounds) in [
        ("float", &[][..], &pair[..1], 100_000),
        ("labelled_float", &LABELS[..1], &pair[..1], 100_000),
        ("many_labels", &labels[..], &pair[..1], 10_000),
        ("two_floats", &[][..], &pair[..], 50_000),
        ("empty", &[][..], &pair[..0], 100_000),
        ("histogram", &[][..], &native[..], 10_000),
    ] {
        let (elapsed, memory, checksum) = measured(labels, points, rounds, false);
        println!(
            "{{\"case\":\"{name}\",\"rounds\":{rounds},\"elapsed_ns\":{elapsed},\"workspace_acquired\":{},\"workspace_released\":{},\"workspace_peak\":{},\"work\":{},\"checksum\":{checksum}}}",
            memory.acquired,
            memory.released,
            memory.peak,
            (labels.len() + 2 * points.len()) as u64 * rounds
        );
    }
}
