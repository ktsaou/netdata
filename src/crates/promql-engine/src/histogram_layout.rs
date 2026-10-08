// SPDX-License-Identifier: GPL-3.0-or-later
// Sparse layout algorithms derive from Prometheus model/histogram; see NOTICE.
use crate::accounting::{AccountedVec, AllocationContext};
use crate::histogram::Span;
use crate::histogram_work::{HistogramEvaluationError, HistogramWork};
use crate::numerical::{compensated_add, source_add};
use crate::storage::Interrupted;
use std::fmt;

#[derive(Debug, Eq, PartialEq)]
pub enum HistogramError {
    IncompatibleSchema,
    CustomSource,
    CustomTarget,
    NonReducing { origin: i32, target: i32 },
    NegativeOffset { span: usize, offset: i32 },
    TooFewBuckets { have: usize },
    TooManyBuckets { need: usize, have: usize },
}

impl HistogramError {
    pub fn is_data_error(&self) -> bool {
        matches!(
            self,
            Self::IncompatibleSchema
                | Self::NegativeOffset { .. }
                | Self::TooFewBuckets { .. }
                | Self::TooManyBuckets { .. }
        )
    }
}

impl fmt::Display for HistogramError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleSchema => out.write_str(
                "cannot apply this operation on histograms with a mix of exponential and custom bucket schemas",
            ),
            Self::CustomSource => {
                out.write_str("cannot reduce resolution when there are custom buckets")
            }
            Self::CustomTarget => {
                out.write_str("cannot reduce resolution to custom buckets schema")
            }
            Self::NonReducing { origin, target } => write!(
                out,
                "cannot reduce resolution from schema {origin} to {target}"
            ),
            Self::NegativeOffset { span, offset } => write!(
                out,
                "span number {span} with offset {offset}: histogram has a span whose offset is negative"
            ),
            Self::TooFewBuckets { have } => write!(
                out,
                "have {have} buckets but spans need more: histogram spans specify different number of buckets than provided"
            ),
            Self::TooManyBuckets { need, have } => write!(
                out,
                "spans need {need} buckets, have {have} buckets: histogram spans specify different number of buckets than provided"
            ),
        }
    }
}

impl std::error::Error for HistogramError {}

trait ReductionBuffer<T: Copy> {
    fn source_len(&self) -> usize;
    fn source(&self, index: usize) -> T;
    fn target(&self, index: usize) -> T;
    fn write(&mut self, index: usize, value: T, work: HistogramWork<'_>)
    -> Result<(), Interrupted>;
}

struct Reused<'a, T>(&'a mut [T]);

impl<T: Copy> ReductionBuffer<T> for Reused<'_, T> {
    fn source_len(&self) -> usize {
        self.0.len()
    }
    fn source(&self, index: usize) -> T {
        self.0[index]
    }
    fn target(&self, index: usize) -> T {
        self.0[index]
    }
    fn write(&mut self, index: usize, value: T, _: HistogramWork<'_>) -> Result<(), Interrupted> {
        self.0[index] = value;
        Ok(())
    }
}

struct Copied<'a, T> {
    source: &'a [T],
    target: Option<AccountedVec<T>>,
    context: Option<&'a AllocationContext>,
}

impl<T: Copy> ReductionBuffer<T> for Copied<'_, T> {
    fn source_len(&self) -> usize {
        self.source.len()
    }
    fn source(&self, index: usize) -> T {
        self.source[index]
    }
    fn target(&self, index: usize) -> T {
        self.target.as_ref().unwrap()[index]
    }
    fn write(
        &mut self,
        index: usize,
        value: T,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        let target = self
            .target
            .get_or_insert_with(|| AccountedVec::new_in(self.context));
        if index == target.len() {
            target.push_copy_with(value, |units| work.step(units))?;
        } else {
            target[index] = value;
        }
        Ok(())
    }
}

pub(crate) fn target_index(index: i32, origin: i32, target: i32) -> i32 {
    let shift = origin.wrapping_sub(target);
    assert!(shift >= 0, "negative shift amount");
    let index = index.wrapping_sub(1);
    let shifted = if shift >= 32 {
        if index < 0 { -1 } else { 0 }
    } else {
        index >> shift
    };
    shifted.wrapping_add(1)
}

fn reduce_float_layout(
    spans: &mut impl ReductionBuffer<Span>,
    buckets: &mut impl ReductionBuffer<f64>,
    origin: i32,
    target: i32,
    work: HistogramWork<'_>,
) -> Result<(usize, usize), HistogramEvaluationError> {
    let (mut written_spans, mut written_buckets) = (0, 0);
    let (mut index, mut last_target): (i32, i32) = (0, 0);
    let mut read_bucket = 0;
    for n in 0..spans.source_len() {
        work.step(1)?;
        // Save the source span before its prefix can be overwritten in-place.
        let span = spans.source(n);
        if n > 0 && span.offset < 0 {
            return Err(HistogramError::NegativeOffset {
                span: n + 1,
                offset: span.offset,
            }
            .into());
        }
        index = index.wrapping_add(span.offset);
        for _ in 0..span.length {
            work.step(1)?;
            if read_bucket >= buckets.source_len() {
                return Err(HistogramError::TooFewBuckets {
                    have: buckets.source_len(),
                }
                .into());
            }
            let target_index = target_index(index, origin, target);
            let value = buckets.source(read_bucket);
            if written_spans == 0 {
                spans.write(
                    0,
                    Span {
                        offset: target_index,
                        length: 1,
                    },
                    work,
                )?;
                buckets.write(0, value, work)?;
                written_spans = 1;
                written_buckets = 1;
                last_target = target_index;
            } else if last_target == target_index {
                let last = written_buckets - 1;
                buckets.write(last, source_add(buckets.target(last), value), work)?;
            } else if last_target.wrapping_add(1) == target_index {
                let last = written_spans - 1;
                let mut span = spans.target(last);
                span.length = span.length.wrapping_add(1);
                spans.write(last, span, work)?;
                last_target = last_target.wrapping_add(1);
                buckets.write(written_buckets, value, work)?;
                written_buckets += 1;
            } else if last_target.wrapping_add(1) < target_index {
                spans.write(
                    written_spans,
                    Span {
                        offset: target_index.wrapping_sub(last_target).wrapping_sub(1),
                        length: 1,
                    },
                    work,
                )?;
                written_spans += 1;
                last_target = target_index;
                buckets.write(written_buckets, value, work)?;
                written_buckets += 1;
            }
            index = index.wrapping_add(1);
            read_bucket += 1;
        }
    }
    if read_bucket != buckets.source_len() {
        return Err(HistogramError::TooManyBuckets {
            need: read_bucket,
            have: buckets.source_len(),
        }
        .into());
    }
    Ok((written_spans, written_buckets))
}

pub(crate) fn reduce_float_in_place(
    spans: &mut Option<AccountedVec<Span>>,
    buckets: &mut Option<AccountedVec<f64>>,
    origin: i32,
    target: i32,
) -> Result<(), HistogramError> {
    reduce_float_in_place_with(spans, buckets, origin, target, HistogramWork::default())
        .map_err(HistogramEvaluationError::arithmetic)
}

pub(crate) fn reduce_float_in_place_with(
    spans: &mut Option<AccountedVec<Span>>,
    buckets: &mut Option<AccountedVec<f64>>,
    origin: i32,
    target: i32,
    work: HistogramWork<'_>,
) -> Result<(), HistogramEvaluationError> {
    // Keep arrays attached during unwinding; only returned errors assign nil outputs.
    let result = reduce_float_layout(
        &mut Reused(spans.as_deref_mut().unwrap_or(&mut [])),
        &mut Reused(buckets.as_deref_mut().unwrap_or(&mut [])),
        origin,
        target,
        work,
    );
    let (span_count, bucket_count) = match result {
        Ok(counts) => counts,
        Err(error) => {
            *spans = None;
            *buckets = None;
            return Err(error);
        }
    };
    if let Some(values) = spans {
        values.truncate(span_count);
    }
    if let Some(values) = buckets {
        values.truncate(bucket_count);
    }
    Ok(())
}

type FloatLayout = (Option<AccountedVec<Span>>, Option<AccountedVec<f64>>);

pub(crate) fn reduce_float_copy_with(
    spans: Option<&[Span]>,
    buckets: Option<&[f64]>,
    origin: i32,
    target: i32,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<FloatLayout, HistogramEvaluationError> {
    let mut spans = Copied {
        source: spans.unwrap_or(&[]),
        target: None,
        context,
    };
    let mut buckets = Copied {
        source: buckets.unwrap_or(&[]),
        target: None,
        context,
    };
    reduce_float_layout(&mut spans, &mut buckets, origin, target, work)?;
    Ok((spans.target, buckets.target))
}

pub(crate) fn compact_float(
    buckets: &mut Option<AccountedVec<f64>>,
    spans: &mut Option<AccountedVec<Span>>,
    max_empty: i64,
    context: Option<&AllocationContext>,
) {
    compact_float_pair(buckets, spans, None, max_empty, context);
}

pub(crate) fn compact_float_pair(
    buckets: &mut Option<AccountedVec<f64>>,
    spans: &mut Option<AccountedVec<Span>>,
    correction: Option<&mut Option<AccountedVec<f64>>>,
    max_empty: i64,
    context: Option<&AllocationContext>,
) {
    let result: Result<(), std::convert::Infallible> =
        compact_float_pair_with(buckets, spans, correction, max_empty, context, |_| Ok(()));
    match result {
        Ok(()) => (),
        Err(never) => match never {},
    }
}

pub(crate) fn compact_float_pair_with<E>(
    buckets: &mut Option<AccountedVec<f64>>,
    spans: &mut Option<AccountedVec<Span>>,
    correction: Option<&mut Option<AccountedVec<f64>>>,
    max_empty: i64,
    context: Option<&AllocationContext>,
    mut poll: impl FnMut(usize) -> Result<(), E>,
) -> Result<(), E> {
    poll(1)?;
    let (mut absent_buckets, mut absent_spans) =
        (AccountedVec::new_in(context), AccountedVec::new_in(context));
    let buckets = buckets.as_mut().unwrap_or(&mut absent_buckets);
    let spans = spans.as_mut().unwrap_or(&mut absent_spans);
    let mut correction = correction.and_then(Option::as_mut);
    if let Some(values) = correction.as_ref() {
        assert_eq!(
            buckets.len(),
            values.len(),
            "compensation bucket layout mismatch"
        );
    }
    let mut has_zero = false;
    for &bucket in buckets.iter() {
        poll(1)?;
        if bucket == 0.0 {
            has_zero = true;
            break;
        }
    }
    if !has_zero {
        let mut unchanged = true;
        for span in spans.iter() {
            poll(1)?;
            if i64::from(span.offset) <= max_empty || span.length == 0 {
                unchanged = false;
                break;
            }
        }
        if unchanged {
            return Ok(());
        }
    }
    let (mut bucket, mut span_index, mut position) = (0, 0, 0u32);
    if spans.len() > 1 {
        for n in 1..spans.len() {
            poll(1)?;
            let span = spans[n];
            if span.offset == 0 {
                spans[span_index].length = spans[span_index].length.wrapping_add(span.length);
            } else {
                span_index += 1;
                spans[span_index] = span;
            }
        }
        spans.truncate(span_index + 1);
        span_index = 0;
    }
    for n in 0..spans.len() {
        poll(1)?;
        let span = spans[n];
        if span.length == 0 {
            if n + 1 < spans.len() {
                spans[n + 1].offset = spans[n + 1].offset.wrapping_add(span.offset);
            }
        } else {
            spans[span_index] = span;
            span_index += 1;
        }
    }
    spans.truncate(span_index);
    span_index = 0;
    if spans.is_empty() {
        buckets.clear();
        if let Some(values) = correction {
            values.clear();
        }
        return Ok(());
    }
    while bucket < buckets.len() && span_index < spans.len() {
        poll(1)?;
        let mut empty = 0usize;
        let mut value = buckets[bucket];
        let mut compensation = correction.as_ref().map_or(0.0, |values| values[bucket]);
        while (empty as u32).wrapping_add(position) < spans[span_index].length
            && value == 0.0
            && compensation == 0.0
        {
            poll(1)?;
            empty += 1;
            if empty + bucket >= buckets.len() {
                break;
            }
            value = buckets[empty + bucket];
            compensation = correction
                .as_ref()
                .map_or(0.0, |values| values[empty + bucket]);
        }
        if empty > 0 {
            if position > 0
                && empty < (spans[span_index].length - position) as usize
                && empty as i64 <= max_empty
            {
                bucket += empty;
                position += empty as u32;
                continue;
            }
            buckets.remove_copy_range_with(bucket, empty, &mut poll)?;
            if let Some(values) = correction.as_mut() {
                values.remove_copy_range_with(bucket, empty, &mut poll)?;
            }
            if position == 0 {
                if empty == spans[span_index].length as usize {
                    let offset = spans[span_index].offset;
                    spans.remove_copy_range_with(span_index, 1, &mut poll)?;
                    if span_index < spans.len() {
                        spans[span_index].offset = spans[span_index]
                            .offset
                            .wrapping_add(offset)
                            .wrapping_add(empty as i32);
                    }
                } else {
                    spans[span_index].length -= empty as u32;
                    spans[span_index].offset = spans[span_index].offset.wrapping_add(empty as i32);
                }
                continue;
            }
            let next = Span {
                offset: empty as i32,
                length: spans[span_index].length - position - empty as u32,
            };
            spans[span_index].length = position;
            span_index += 1;
            position = 0;
            if next.length == 0 {
                if span_index < spans.len() {
                    spans[span_index].offset = spans[span_index].offset.wrapping_add(empty as i32);
                }
            } else {
                spans.insert_copy_with(span_index, next, &mut poll)?;
            }
            continue;
        }
        bucket += 1;
        position += 1;
        if position >= spans[span_index].length {
            position = 0;
            span_index += 1;
        }
    }
    if max_empty == 0 || buckets.is_empty() {
        return Ok(());
    }
    bucket = spans[0].length as usize;
    span_index = 1;
    while span_index < spans.len() {
        poll(1)?;
        let span = spans[span_index];
        if i64::from(span.offset) > max_empty {
            bucket += span.length as usize;
            span_index += 1;
            continue;
        }
        let offset = usize::try_from(span.offset).expect("negative bucket gap");
        spans[span_index - 1].length = spans[span_index - 1]
            .length
            .wrapping_add(offset as u32)
            .wrapping_add(span.length);
        spans.remove_copy_range_with(span_index, 1, &mut poll)?;
        let old_len = buckets.len();
        // Reuse this payload instead of holding an additional complete bucket array.
        buckets.resize_copy_with(old_len + offset, 0.0, &mut poll)?;
        buckets.copy_within_with(bucket..old_len, bucket + offset, &mut poll)?;
        for chunk in buckets[bucket..bucket + offset].chunks_mut(1024) {
            poll(chunk.len())?;
            chunk.fill(0.0);
        }
        if let Some(values) = correction.as_mut() {
            let old_len = values.len();
            values.resize_copy_with(old_len + offset, 0.0, &mut poll)?;
            values.copy_within_with(bucket..old_len, bucket + offset, &mut poll)?;
            for chunk in values[bucket..bucket + offset].chunks_mut(1024) {
                poll(chunk.len())?;
                chunk.fill(0.0);
            }
        }
        bucket += offset + span.length as usize;
    }
    Ok(())
}

fn reduce_kahan_layout(
    spans: &mut impl ReductionBuffer<Span>,
    buckets: &mut impl ReductionBuffer<f64>,
    correction: &mut impl ReductionBuffer<f64>,
    origin: i32,
    target: i32,
    work: HistogramWork<'_>,
) -> Result<(usize, usize), Interrupted> {
    work.step(1)?;
    let (mut written_spans, mut written_buckets) = (0, 0);
    let (mut index, mut last_target): (i32, i32) = (0, 0);
    let mut read_bucket = 0;
    for n in 0..spans.source_len() {
        work.step(1)?;
        let span = spans.source(n);
        index = index.wrapping_add(span.offset);
        for _ in 0..span.length {
            work.step(1)?;
            let target = target_index(index, origin, target);
            if written_spans == 0 {
                spans.write(
                    0,
                    Span {
                        offset: target,
                        length: 1,
                    },
                    work,
                )?;
                buckets.write(0, buckets.source(read_bucket), work)?;
                last_target = target;
                correction.write(0, correction.source(read_bucket), work)?;
                written_spans = 1;
                written_buckets = 1;
            } else if last_target == target {
                let last = written_buckets - 1;
                let (sum, comp) = compensated_add(
                    buckets.source(read_bucket),
                    buckets.target(last),
                    correction.target(last),
                );
                buckets.write(last, sum, work)?;
                correction.write(last, comp, work)?;
                let (sum, comp) = compensated_add(
                    correction.source(read_bucket),
                    buckets.target(last),
                    correction.target(last),
                );
                buckets.write(last, sum, work)?;
                correction.write(last, comp, work)?;
            } else if last_target.wrapping_add(1) == target {
                let last = written_spans - 1;
                let mut span = spans.target(last);
                span.length = span.length.wrapping_add(1);
                spans.write(last, span, work)?;
                last_target = last_target.wrapping_add(1);
                buckets.write(written_buckets, buckets.source(read_bucket), work)?;
                correction.write(written_buckets, correction.source(read_bucket), work)?;
                written_buckets += 1;
            } else if last_target.wrapping_add(1) < target {
                spans.write(
                    written_spans,
                    Span {
                        offset: target.wrapping_sub(last_target).wrapping_sub(1),
                        length: 1,
                    },
                    work,
                )?;
                written_spans += 1;
                last_target = target;
                buckets.write(written_buckets, buckets.source(read_bucket), work)?;
                correction.write(written_buckets, correction.source(read_bucket), work)?;
                written_buckets += 1;
            }
            index = index.wrapping_add(1);
            read_bucket += 1;
        }
    }
    work.step(1)?;
    Ok((written_spans, written_buckets))
}

pub(crate) fn reduce_kahan_in_place_with(
    spans: &mut Option<AccountedVec<Span>>,
    buckets: &mut Option<AccountedVec<f64>>,
    correction: &mut Option<AccountedVec<f64>>,
    origin: i32,
    target: i32,
    work: HistogramWork<'_>,
) -> Result<(), Interrupted> {
    let (span_count, bucket_count) = reduce_kahan_layout(
        &mut Reused(spans.as_deref_mut().unwrap_or(&mut [])),
        &mut Reused(buckets.as_deref_mut().unwrap_or(&mut [])),
        &mut Reused(correction.as_deref_mut().unwrap_or(&mut [])),
        origin,
        target,
        work,
    )?;
    if let Some(values) = spans {
        values.truncate(span_count);
    }
    if let Some(values) = buckets {
        values.truncate(bucket_count);
    }
    if let Some(values) = correction {
        values.truncate(bucket_count);
    }
    work.step(1)
}

struct ZeroCopied<'a> {
    source_len: usize,
    target: Option<AccountedVec<f64>>,
    context: Option<&'a AllocationContext>,
}

impl ReductionBuffer<f64> for ZeroCopied<'_> {
    fn source_len(&self) -> usize {
        self.source_len
    }
    fn source(&self, index: usize) -> f64 {
        assert!(index < self.source_len);
        0.0
    }
    fn target(&self, index: usize) -> f64 {
        self.target.as_ref().unwrap()[index]
    }
    fn write(
        &mut self,
        index: usize,
        value: f64,
        work: HistogramWork<'_>,
    ) -> Result<(), Interrupted> {
        let target = self
            .target
            .get_or_insert_with(|| AccountedVec::new_in(self.context));
        if index == target.len() {
            target.push_copy_with(value, |units| work.step(units))?;
        } else {
            target[index] = value;
        }
        Ok(())
    }
}

pub(crate) struct ReducedKahan {
    pub spans: Option<AccountedVec<Span>>,
    pub buckets: Option<AccountedVec<f64>>,
    pub compensation: Option<AccountedVec<f64>>,
}

pub(crate) fn reduce_kahan_copy_with(
    spans: Option<&[Span]>,
    buckets: Option<&[f64]>,
    origin: i32,
    target: i32,
    context: Option<&AllocationContext>,
    work: HistogramWork<'_>,
) -> Result<ReducedKahan, Interrupted> {
    let mut spans = Copied {
        source: spans.unwrap_or(&[]),
        target: None,
        context,
    };
    let mut buckets = Copied {
        source: buckets.unwrap_or(&[]),
        target: None,
        context,
    };
    // Read the upstream zero correction virtually instead of allocating its unreduced array.
    let mut correction = ZeroCopied {
        source_len: buckets.source.len(),
        target: None,
        context,
    };
    reduce_kahan_layout(
        &mut spans,
        &mut buckets,
        &mut correction,
        origin,
        target,
        work,
    )?;
    Ok(ReducedKahan {
        spans: spans.target,
        buckets: buckets.target,
        compensation: correction.target,
    })
}
