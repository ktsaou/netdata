// SPDX-License-Identifier: GPL-3.0-or-later
// Sparse layout algorithms derive from Prometheus model/histogram; see NOTICE.
use crate::histogram::Span;
use std::fmt;

#[derive(Debug, Eq, PartialEq)]
pub enum HistogramError {
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
            Self::NegativeOffset { .. } | Self::TooFewBuckets { .. } | Self::TooManyBuckets { .. }
        )
    }
}

impl fmt::Display for HistogramError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
    fn write(&mut self, index: usize, value: T);
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
    fn write(&mut self, index: usize, value: T) {
        self.0[index] = value;
    }
}

struct Copied<'a, T> {
    source: &'a [T],
    target: Option<Vec<T>>,
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
    fn write(&mut self, index: usize, value: T) {
        let target = self.target.get_or_insert_default();
        if index == target.len() {
            target.push(value);
        } else {
            target[index] = value;
        }
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
) -> Result<(usize, usize), HistogramError> {
    let (mut written_spans, mut written_buckets) = (0, 0);
    let (mut index, mut last_target): (i32, i32) = (0, 0);
    let mut read_bucket = 0;
    for n in 0..spans.source_len() {
        // Save the source span before its prefix can be overwritten in-place.
        let span = spans.source(n);
        if n > 0 && span.offset < 0 {
            return Err(HistogramError::NegativeOffset {
                span: n + 1,
                offset: span.offset,
            });
        }
        index = index.wrapping_add(span.offset);
        for _ in 0..span.length {
            if read_bucket >= buckets.source_len() {
                return Err(HistogramError::TooFewBuckets {
                    have: buckets.source_len(),
                });
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
                );
                buckets.write(0, value);
                written_spans = 1;
                written_buckets = 1;
                last_target = target_index;
            } else if last_target == target_index {
                let last = written_buckets - 1;
                buckets.write(last, buckets.target(last) + value);
            } else if last_target.wrapping_add(1) == target_index {
                let last = written_spans - 1;
                let mut span = spans.target(last);
                span.length = span.length.wrapping_add(1);
                spans.write(last, span);
                last_target = last_target.wrapping_add(1);
                buckets.write(written_buckets, value);
                written_buckets += 1;
            } else if last_target.wrapping_add(1) < target_index {
                spans.write(
                    written_spans,
                    Span {
                        offset: target_index.wrapping_sub(last_target).wrapping_sub(1),
                        length: 1,
                    },
                );
                written_spans += 1;
                last_target = target_index;
                buckets.write(written_buckets, value);
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
        });
    }
    Ok((written_spans, written_buckets))
}

pub(crate) fn reduce_float_in_place(
    spans: &mut Option<Vec<Span>>,
    buckets: &mut Option<Vec<f64>>,
    origin: i32,
    target: i32,
) -> Result<(), HistogramError> {
    // Keep arrays attached during unwinding; only returned errors assign nil outputs.
    let result = reduce_float_layout(
        &mut Reused(spans.as_deref_mut().unwrap_or(&mut [])),
        &mut Reused(buckets.as_deref_mut().unwrap_or(&mut [])),
        origin,
        target,
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

pub(crate) fn reduce_float_copy(
    spans: Option<&[Span]>,
    buckets: Option<&[f64]>,
    origin: i32,
    target: i32,
) -> (Option<Vec<Span>>, Option<Vec<f64>>) {
    let mut spans = Copied {
        source: spans.unwrap_or(&[]),
        target: None,
    };
    let mut buckets = Copied {
        source: buckets.unwrap_or(&[]),
        target: None,
    };
    if let Err(error) = reduce_float_layout(&mut spans, &mut buckets, origin, target) {
        panic!("{error}");
    }
    (spans.target, buckets.target)
}

fn remove_range<T: Copy>(values: &mut Vec<T>, start: usize, length: usize) {
    values.copy_within(start + length.., start);
    values.truncate(values.len() - length);
}

pub(crate) fn compact_float(
    buckets: &mut Option<Vec<f64>>,
    spans: &mut Option<Vec<Span>>,
    max_empty: isize,
) {
    let (mut absent_buckets, mut absent_spans) = (Vec::new(), Vec::new());
    let buckets = buckets.as_mut().unwrap_or(&mut absent_buckets);
    let spans = spans.as_mut().unwrap_or(&mut absent_spans);
    if !buckets.contains(&0.0)
        && spans
            .iter()
            .all(|span| span.offset as isize > max_empty && span.length != 0)
    {
        return;
    }
    let (mut bucket, mut span_index, mut position) = (0, 0, 0u32);
    if spans.len() > 1 {
        for n in 1..spans.len() {
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
        return;
    }
    while bucket < buckets.len() && span_index < spans.len() {
        let mut empty = 0usize;
        let mut value = buckets[bucket];
        while (empty as u32).wrapping_add(position) < spans[span_index].length && value == 0.0 {
            empty += 1;
            if empty + bucket >= buckets.len() {
                break;
            }
            value = buckets[empty + bucket];
        }
        if empty > 0 {
            if position > 0
                && empty < (spans[span_index].length - position) as usize
                && empty as isize <= max_empty
            {
                bucket += empty;
                position += empty as u32;
                continue;
            }
            remove_range(buckets, bucket, empty);
            if position == 0 {
                if empty == spans[span_index].length as usize {
                    let offset = spans[span_index].offset;
                    spans.remove(span_index);
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
                spans.insert(span_index, next);
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
        return;
    }
    bucket = spans[0].length as usize;
    span_index = 1;
    while span_index < spans.len() {
        let span = spans[span_index];
        if span.offset as isize > max_empty {
            bucket += span.length as usize;
            span_index += 1;
            continue;
        }
        let offset = usize::try_from(span.offset).expect("negative bucket gap");
        spans[span_index - 1].length = spans[span_index - 1]
            .length
            .wrapping_add(offset as u32)
            .wrapping_add(span.length);
        spans.remove(span_index);
        let old_len = buckets.len();
        // Reuse this payload instead of holding an additional complete bucket array.
        buckets.resize(old_len + offset, 0.0);
        buckets.copy_within(bucket..old_len, bucket + offset);
        buckets[bucket..bucket + offset].fill(0.0);
        bucket += offset + span.length as usize;
    }
}
