// SPDX-License-Identifier: GPL-3.0-or-later
// Statistics decoding and access epochs derive from pinned PromQL/storage; see NOTICE.
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    frontend::{
        ast::{Expr, ParsedExpression},
        token as tokens,
    },
    histogram::{COUNTER_RESET, FloatHistogramRef, NOT_RESET},
    result::{OwnedHistogram, OwnedValue},
    storage::{Control, Interrupted, Series, StorageError, TimeBounds, scan},
    value::{SampleRef, SampleValueRef},
};
use tokens::TokenKind;

pub(crate) fn configure(
    expression: &mut ParsedExpression,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), Interrupted> {
    let mut tasks = AccountedVec::new(account, MemoryKind::Workspace);
    tasks.push((expression.root, false, false));
    while let Some((node, mut enabled, mut blocked)) = tasks.pop() {
        control.check()?;
        account.work(1);
        if let Expr::Selector(selector) = &mut expression.arena[node].expression {
            selector.skip_histogram_buckets = enabled && !blocked;
            continue;
        }
        match &expression.arena[node].expression {
            Expr::Call {
                function,
                arguments,
            } => {
                if let Some(function) = function {
                    match function.name {
                        "histogram_count" | "histogram_sum" | "histogram_avg" => enabled = true,
                        "histogram_quantile"
                        | "histogram_quantiles"
                        | "histogram_fraction"
                        | "histogram_stddev"
                        | "histogram_stdvar" => blocked = true,
                        _ => (),
                    }
                }
                tasks.extend(
                    arguments
                        .iter()
                        .rev()
                        .map(|&child| (child, enabled, blocked)),
                );
            }
            Expr::Binary {
                operator,
                left,
                right,
                ..
            } => {
                blocked |= matches!(*operator, TokenKind::TRIM_UPPER | TokenKind::TRIM_LOWER);
                tasks.push((*right, enabled, blocked));
                tasks.push((*left, enabled, blocked));
            }
            Expr::Aggregate {
                parameter,
                expression,
                ..
            }
            | Expr::Duration {
                left: parameter,
                right: expression,
                ..
            } => {
                tasks.extend(
                    parameter
                        .iter()
                        .chain(expression.iter())
                        .map(|&child| (child, enabled, blocked)),
                );
            }
            Expr::Paren(child)
            | Expr::Unary {
                expression: child, ..
            } => tasks.push((*child, enabled, blocked)),
            Expr::Matrix {
                selector,
                range_expression,
                ..
            } => {
                tasks.push((*selector, enabled, blocked));
                tasks.extend(
                    range_expression
                        .iter()
                        .map(|&child| (child, enabled, blocked)),
                );
            }
            Expr::Subquery {
                expression,
                range_expression,
                step_expression,
                ..
            } => {
                tasks.push((*expression, enabled, true));
                tasks.extend(
                    range_expression
                        .iter()
                        .chain(step_expression.iter())
                        .map(|&child| (child, enabled, true)),
                );
            }
            _ => (),
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct StatisticsWindow {
    pub(crate) first_end_ms: i64,
    pub(crate) range_ms: i64,
    pub(crate) step_ms: i64,
}

/// Reconstruct accessed histograms without retaining the source iterator's raw window.
pub(crate) fn scan_statistics<S: Series>(
    series: &S,
    bounds: TimeBounds,
    statistics: Option<StatisticsWindow>,
    control: &Control,
    account: &QueryAccounting,
    mut visit: impl for<'a> FnMut(SampleRef<'a>) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let Some(statistics) = statistics else {
        return scan(series, bounds, control, account, visit);
    };
    let mut step_end = statistics.first_end_ms;
    let mut endpoint = None;
    let mut last = None::<OwnedHistogram>;
    let read_bounds = TimeBounds {
        start_ms: statistics
            .first_end_ms
            .wrapping_sub(statistics.range_ms)
            .wrapping_add(1),
        end_ms: bounds.end_ms,
    };
    scan(series, read_bounds, control, account, |sample| {
        let mut position = endpoint.take().unwrap_or(sample.time_ms);
        while sample.time_ms > step_end && step_end < bounds.end_ms {
            control.check()?;
            account.work(1);
            step_end = step_end
                .checked_add(statistics.step_ms)
                .filter(|&next| next > step_end)
                .unwrap_or(bounds.end_ms)
                .min(bounds.end_ms);
            if step_end.wrapping_sub(statistics.range_ms) > position {
                last = None;
            }
            position = sample.time_ms;
        }
        if sample.time_ms < step_end.wrapping_sub(statistics.range_ms) {
            last = None;
            return Ok(());
        }
        if sample.time_ms == step_end {
            endpoint = Some(sample.time_ms);
        }
        let converted;
        let histogram = match sample.value {
            SampleValueRef::Float(_) => {
                return if sample.time_ms >= bounds.start_ms {
                    visit(sample)
                } else {
                    Ok(())
                };
            }
            SampleValueRef::FloatHistogram(histogram) => histogram,
            SampleValueRef::IntegerHistogram(_) => {
                let OwnedValue::Histogram(histogram) =
                    OwnedValue::copy_from(sample.value, account, MemoryKind::Workspace)
                else {
                    unreachable!()
                };
                converted = histogram;
                converted.as_ref()
            }
        };
        let mut hint = histogram.reset_hint;
        if !sample.value.is_stale() {
            if hint == 0
                && let Some(previous) = &last
            {
                hint = if histogram.detect_reset(previous.as_ref()) {
                    COUNTER_RESET
                } else {
                    NOT_RESET
                };
            }
            let OwnedValue::Histogram(mut retained) = OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(histogram),
                account,
                MemoryKind::Workspace,
            ) else {
                unreachable!()
            };
            retained.set_reset_hint(hint);
            last = Some(retained);
        }
        if sample.time_ms < bounds.start_ms {
            return Ok(());
        }
        visit(SampleRef {
            value: SampleValueRef::FloatHistogram(FloatHistogramRef {
                reset_hint: hint,
                schema: histogram.schema,
                count: histogram.count,
                sum: histogram.sum,
                zero_threshold: 0.0,
                zero_count: 0.0,
                positive_spans: None,
                negative_spans: None,
                positive_buckets: None,
                negative_buckets: None,
                custom_values: None,
            }),
            ..sample
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::FloatHistogram,
        test_storage::GeneratedSeries,
        value::{STALE_NAN_BITS, Sample, SampleValue},
    };

    fn histogram(time_ms: i64, count: f64, hint: u8) -> Sample {
        Sample {
            time_ms,
            start_time_ms: 0,
            value: SampleValue::FloatHistogram(FloatHistogram {
                reset_hint: hint,
                count,
                sum: count,
                zero_count: count,
                zero_threshold: 0.001,
                ..FloatHistogram::default()
            }),
        }
    }

    #[test]
    fn buffered_replay_preserves_lookahead_and_clears_moving_seeks() {
        fn lookahead(index: usize) -> Sample {
            histogram([900, 5000][index], [8.0, 4.0][index], [NOT_RESET, 0][index])
        }
        fn endpoint(index: usize) -> Sample {
            histogram(
                [1000, 5000][index],
                [8.0, 4.0][index],
                [NOT_RESET, 0][index],
            )
        }
        for page in [1, 17, 1024] {
            for (sample, expected) in [
                (lookahead as fn(usize) -> Sample, COUNTER_RESET),
                (endpoint, 0),
            ] {
                let source = GeneratedSeries::new(2, page, sample);
                let engine = EngineAccounting::default();
                let account = engine.query();
                let mut hints = Vec::new();
                scan_statistics(
                    &source,
                    TimeBounds {
                        start_ms: 4501,
                        end_ms: 5000,
                    },
                    Some(StatisticsWindow {
                        first_end_ms: 1000,
                        range_ms: 500,
                        step_ms: 4000,
                    }),
                    &Control::default(),
                    &account,
                    |sample| {
                        let SampleValueRef::FloatHistogram(histogram) = sample.value else {
                            unreachable!()
                        };
                        hints.push(histogram.reset_hint);
                        assert_eq!(histogram.count, 4.0);
                        assert_eq!(histogram.zero_count, 0.0);
                        assert_eq!(histogram.zero_threshold, 0.0);
                        assert!(histogram.positive_buckets.is_none());
                        assert!(histogram.custom_values.is_none());
                        Ok(())
                    },
                )
                .unwrap();
                assert_eq!(hints, [expected]);
                assert!(
                    account
                        .snapshot()
                        .memory
                        .iter()
                        .all(|memory| memory.current == 0)
                );
                assert_eq!(
                    source
                        .counters
                        .opened
                        .load(std::sync::atomic::Ordering::Relaxed),
                    source
                        .counters
                        .closed
                        .load(std::sync::atomic::Ordering::Relaxed)
                );
            }
        }
    }

    #[test]
    fn floats_and_stale_histograms_preserve_the_last_accessed_histogram() {
        fn sample(index: usize) -> Sample {
            match index {
                0 => histogram(1000, 8.0, NOT_RESET),
                1 => Sample {
                    time_ms: 2000,
                    start_time_ms: 0,
                    value: SampleValue::Float(100.0),
                },
                2 => {
                    let mut sample = histogram(3000, 0.0, 0);
                    let SampleValue::FloatHistogram(histogram) = &mut sample.value else {
                        unreachable!()
                    };
                    histogram.sum = f64::from_bits(STALE_NAN_BITS);
                    sample
                }
                _ => histogram(4000, 4.0, 0),
            }
        }
        let source = GeneratedSeries::new(4, 1, sample);
        let engine = EngineAccounting::default();
        let account = engine.query();
        let mut hints = Vec::new();
        scan_statistics(
            &source,
            TimeBounds {
                start_ms: 3001,
                end_ms: 4000,
            },
            Some(StatisticsWindow {
                first_end_ms: 1000,
                range_ms: 1000,
                step_ms: 1000,
            }),
            &Control::default(),
            &account,
            |sample| {
                let SampleValueRef::FloatHistogram(histogram) = sample.value else {
                    unreachable!()
                };
                hints.push(histogram.reset_hint);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(hints, [COUNTER_RESET]);
        assert!(
            account
                .snapshot()
                .memory
                .iter()
                .all(|memory| memory.current == 0)
        );
    }
}
