// SPDX-License-Identifier: GPL-3.0-or-later
// Count labels follow pinned model/histogram/{float_histogram,generic}.go; see NOTICE.
use crate::{
    accounting::{AccountedVec, QueryAccounting},
    evaluator::EvaluationError,
    float_text::{FloatTextStyle, float_text},
    histogram::{Bucket, FloatHistogramRef},
    histogram_work::HistogramWork,
    result::append_bytes_checked,
    storage::{Control, Interrupted},
};

fn number(
    output: &mut AccountedVec<u8>,
    value: f64,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    append_bytes_checked(
        output,
        float_text(value, FloatTextStyle::Shortest).as_bytes(),
        account,
        control,
    )
}
fn bucket(
    output: &mut AccountedVec<u8>,
    value: Bucket,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    append_bytes_checked(
        output,
        if value.lower_inclusive {
            b", ["
        } else {
            b", ("
        },
        account,
        control,
    )?;
    number(output, value.lower, control, account)?;
    append_bytes_checked(output, b",", account, control)?;
    number(output, value.upper, control, account)?;
    append_bytes_checked(
        output,
        if value.upper_inclusive { b"]:" } else { b"):" },
        account,
        control,
    )?;
    number(output, value.count, control, account)
}

pub(crate) fn append(
    output: &mut AccountedVec<u8>,
    histogram: FloatHistogramRef<'_>,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    let work = HistogramWork::new(control, account);
    work.step(1)?;
    append_bytes_checked(output, b"{count:", account, control)?;
    number(output, histogram.count, control, account)?;
    append_bytes_checked(output, b", sum:", account, control)?;
    number(output, histogram.sum, control, account)?;
    let mut negative = histogram.bucket_iterator_with(false, 0.0, histogram.schema, work)?;
    let mut reversed = None;
    while negative.advance_with(work)? {
        let _ = negative.at_with(work)?;
        reversed = Some(negative.reverse_productive_prefix());
    }
    if let Some(mut reversed) = reversed {
        while reversed.advance_with(work)? {
            let value = reversed.at_with(work)?;
            if value.count != 0.0 {
                bucket(output, value, control, account)?;
            }
        }
    }
    if histogram.zero_count != 0.0 {
        bucket(output, histogram.zero_bucket_with(work)?, control, account)?;
    }
    let mut positive = histogram.bucket_iterator_with(true, 0.0, histogram.schema, work)?;
    while positive.advance_with(work)? {
        let value = positive.at_with(work)?;
        if value.count != 0.0 {
            bucket(output, value, control, account)?;
        }
    }
    append_bytes_checked(output, b"}", account, control)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        histogram::{FloatHistogram, Span},
        test_storage::interrupt_during,
    };

    #[test]
    fn negative_text_uses_only_the_productive_prefix_and_unsigned_span_position() {
        for (length, counts) in [
            (3, vec![1.0, 0.0]),
            (1, vec![1.0, 5.0]),
            (u32::MAX, vec![1.0]),
        ] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let histogram = FloatHistogram {
                count: 1.0,
                sum: 1.0,
                negative_spans: Some(vec![Span { offset: -1, length }].into()),
                negative_buckets: Some(counts.into()),
                ..FloatHistogram::default()
            };
            let mut output = AccountedVec::new(&account, MemoryKind::Workspace);
            append(
                &mut output,
                histogram.as_ref(),
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(&*output, b"{count:1, sum:1, [-0.5,-0.25):1}");
            drop(output);
            account.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn omitted_negative_buckets_do_not_allocate_a_reversal_vector_and_remain_interruptible() {
        let engine = EngineAccounting::default();
        let mut peaks = Vec::new();
        for length in [1024, 100_000] {
            let account = engine.query();
            let histogram = FloatHistogram {
                count: 1.0,
                sum: 1.0,
                negative_spans: Some(vec![Span { offset: -1, length }].into()),
                negative_buckets: Some(vec![0.0; length as usize].into()),
                ..FloatHistogram::default()
            };
            let mut output = AccountedVec::new(&account, MemoryKind::Workspace);
            append(
                &mut output,
                histogram.as_ref(),
                &Control::default(),
                &account,
            )
            .unwrap();
            assert_eq!(&*output, b"{count:1, sum:1}");
            peaks.push(account.snapshot().memory[MemoryKind::Workspace as usize].peak);
            drop(output);
            account.snapshot().assert_memory_released();
            let account = engine.query();
            let control = Control::default();
            if length == 100_000 {
                let mut output = AccountedVec::new(&account, MemoryKind::Workspace);
                let outcome = interrupt_during(&account, &control, || {
                    append(&mut output, histogram.as_ref(), &control, &account)
                });
                assert!(matches!(
                    outcome,
                    Err(EvaluationError::Interrupted(Interrupted::Cancelled))
                ));
                drop(output);
                account.snapshot().assert_memory_released();
            }
        }
        assert_eq!(peaks[0], peaks[1]);
        engine.snapshot().assert_memory_released();
    }
}
