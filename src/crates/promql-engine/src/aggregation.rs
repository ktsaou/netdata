// SPDX-License-Identifier: GPL-3.0-or-later
// Per-step grouped reducers derive from pinned PromQL aggregation; see NOTICE.
use crate::{
    accounting::{AccountedBox, AllocationContext, MemoryKind, QueryAccounting},
    annotations::{AnnotationKind, HistogramOperation},
    evaluator::EvaluatedSample,
    evaluator_roster::GroupDescriptor,
    frontend::token as tokens,
    histogram::HistogramError,
    histogram_accumulator::{Finalization, NativeAccumulator},
    histogram_work::{HistogramEvaluationError, HistogramWork},
    numerical::{compensated_add, source_add, source_div, source_mul, source_sub, sqrt},
    result::{OwnedLabels, OwnedValue},
    storage::{Control, StorageError},
};
use tokens::TokenKind;

const NAN: f64 = f64::from_bits(0x7ff8_0000_0000_0001);

#[derive(Debug)]
pub(crate) struct Group<'a> {
    labels: &'a OwnedLabels,
    ordinal: usize,
    pub(crate) hash: u64,
    pub(crate) seen: bool,
    value: f64,
    value_samples: Option<crate::statistics::OwnedSamples>,
    compensation: f64,
    mean: f64,
    count: f64,
    incremental_mean: bool,
    drop_name: bool,
    whole_drop_name: Option<bool>,
    has_float: bool,
    has_histogram: bool,
    incompatible: bool,
    native: Option<AccountedBox<NativeAccumulator>>,
}

impl<'a> Group<'a> {
    pub(crate) fn new(group: &'a GroupDescriptor, ordinal: usize) -> Self {
        Self {
            labels: &group.labels,
            hash: group.hash,
            ordinal,
            seen: false,
            value: 0.0,
            value_samples: None,
            compensation: 0.0,
            mean: 0.0,
            count: 0.0,
            incremental_mean: false,
            drop_name: false,
            whole_drop_name: group.whole_drop_name,
            has_float: false,
            has_histogram: false,
            incompatible: false,
            native: None,
        }
    }
    pub(crate) fn add(
        &mut self,
        operator: TokenKind,
        value: &OwnedValue,
        drop_name: bool,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<Option<AnnotationKind>, StorageError> {
        use TokenKind::*;
        let (float, histogram) = match value {
            OwnedValue::Float(value) => (*value, None),
            OwnedValue::Histogram(value) => (0.0, Some(value)),
        };
        if !self.seen {
            self.seen = true;
            self.value = float;
            self.mean = float;
            self.count = 1.0;
            self.drop_name = drop_name;
            match operator {
                SUM | AVG => {
                    if let Some(histogram) = histogram {
                        self.native = Some(AccountedBox::new(
                            NativeAccumulator::new(
                                operator == AVG,
                                histogram.copy_checked(control, account)?,
                            ),
                            &AllocationContext::new(account, MemoryKind::Workspace),
                        ));
                        self.has_histogram = true;
                    } else {
                        self.has_float = true;
                    }
                }
                MIN | MAX | STDVAR | STDDEV if histogram.is_some() => {
                    self.seen = false;
                    return Ok(Some(AnnotationKind::HistogramIgnoredInAggregation(
                        operator,
                    )));
                }
                STDVAR | STDDEV => {
                    self.value = if float.is_nan() || float.is_infinite() {
                        NAN
                    } else {
                        0.0
                    };
                }
                GROUP => self.value = 1.0,
                MIN | MAX => {
                    self.value_samples = Some(crate::statistics::OwnedSamples::new(account, 1))
                }
                _ => (),
            }
            return Ok(None);
        }
        // Upstream stops a schema-invalid group before consuming later flags or sample kinds.
        if self.incompatible {
            return Ok(None);
        }
        self.drop_name |= drop_name;
        match operator {
            SUM | AVG => {
                if operator == AVG {
                    self.count += 1.0;
                }
                if let Some(histogram) = histogram {
                    self.has_histogram = true;
                    if let Some(native) = &mut self.native {
                        return match native.add(histogram.as_ref(), self.count, control, account) {
                            Ok(true) => Ok(Some(AnnotationKind::MismatchedCustomBuckets(
                                HistogramOperation::Aggregation,
                            ))),
                            Ok(false) => Ok(None),
                            Err(HistogramEvaluationError::Arithmetic(
                                HistogramError::IncompatibleSchema,
                            )) => {
                                self.incompatible = true;
                                Ok(Some(AnnotationKind::MixedExponentialCustomHistograms))
                            }
                            Err(HistogramEvaluationError::Arithmetic(error)) => {
                                Err(HistogramWork::new(control, account)
                                    .source(format_args!("{error}"), false)
                                    .into())
                            }
                            Err(HistogramEvaluationError::Interrupted(error)) => Err(error.into()),
                            Err(HistogramEvaluationError::Source(error)) => Err(error),
                        };
                    }
                } else {
                    self.has_float = true;
                    if operator == SUM {
                        (self.value, self.compensation) =
                            compensated_add(float, self.value, self.compensation);
                    } else {
                        if !self.incremental_mean {
                            let (next, compensation) =
                                compensated_add(float, self.value, self.compensation);
                            if !next.is_infinite() {
                                self.value = next;
                                self.compensation = compensation;
                                return Ok(None);
                            }
                            self.incremental_mean = true;
                            self.mean = source_div(self.value, source_sub(self.count, 1.0));
                            self.compensation =
                                source_div(self.compensation, source_sub(self.count, 1.0));
                        }
                        let q = source_div(source_sub(self.count, 1.0), self.count);
                        (self.mean, self.compensation) = compensated_add(
                            source_div(float, self.count),
                            source_mul(self.mean, q),
                            source_mul(q, self.compensation),
                        );
                    }
                }
            }
            COUNT => self.count += 1.0,
            MIN | MAX | STDVAR | STDDEV if histogram.is_some() => {
                return Ok(Some(AnnotationKind::HistogramIgnoredInAggregation(
                    operator,
                )));
            }
            MIN => {
                if self.value > float || self.value.is_nan() {
                    self.value = float;
                }
            }
            MAX => {
                if self.value < float || self.value.is_nan() {
                    self.value = float;
                }
            }
            STDVAR | STDDEV => {
                self.count += 1.0;
                let delta = source_sub(float, self.mean);
                self.mean = source_add(source_div(delta, self.count), self.mean);
                self.value =
                    source_add(source_mul(source_sub(float, self.mean), delta), self.value);
            }
            GROUP => (),
            _ => unreachable!("qualified aggregation operator"),
        }
        Ok(None)
    }
    pub(crate) fn finish(
        mut self,
        operator: TokenKind,
        time_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(Option<EvaluatedSample>, Option<AnnotationKind>), StorageError> {
        use TokenKind::*;
        if matches!(operator, SUM | AVG) {
            if self.has_float && self.has_histogram {
                return Ok((None, Some(AnnotationKind::MixedFloatsHistogramsAggregation)));
            }
            if self.incompatible {
                return Ok((None, None));
            }
        }
        let (value, annotation) = if self.has_histogram {
            let native = self
                .native
                .take()
                .expect("live histogram group")
                .into_inner();
            let mut result = match native.finish(self.count, Finalization::Vector, control, account)
            {
                Ok(result) => result,
                Err(HistogramEvaluationError::Interrupted(error)) => return Err(error.into()),
                Err(HistogramEvaluationError::Source(error)) => return Err(error),
                Err(HistogramEvaluationError::Arithmetic(error)) => {
                    return Err(HistogramWork::new(control, account)
                        .source(format_args!("{error}"), false)
                        .into());
                }
            };
            result.value.compact_checked(control, account)?;
            (
                OwnedValue::Histogram(result.value),
                result
                    .reset_collision
                    .then_some(AnnotationKind::HistogramCounterResetCollision(
                        HistogramOperation::Aggregation,
                    )),
            )
        } else {
            let value = match operator {
                SUM => source_add(self.value, self.compensation),
                AVG if self.incremental_mean => source_add(self.mean, self.compensation),
                AVG => source_add(
                    source_div(self.value, self.count),
                    source_div(self.compensation, self.count),
                ),
                COUNT => self.count,
                GROUP => 1.0,
                STDVAR => source_div(self.value, self.count),
                STDDEV => sqrt(source_div(self.value, self.count)),
                _ => self.value,
            };
            (OwnedValue::Float(value), None)
        };
        let mut labels = self
            .labels
            .copy_checked(account, MemoryKind::Workspace, control)?;
        // Source removes empty label values only after hashing the raw grouping key.
        labels.reset_checked(account, control)?;
        Ok((
            Some(EvaluatedSample {
                ordinal: self.ordinal,
                labels,
                float_samples: self
                    .value_samples
                    .take()
                    .or_else(|| crate::statistics::hold_float(account, value.as_ref())),
                value,
                source_time_ms: time_ms,
                source_start_time_ms: 0,
                drop_name: self.whole_drop_name.unwrap_or(self.drop_name),
            }),
            annotation,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::EngineAccounting,
        histogram::{FloatHistogram, Span},
        value::SampleValueRef,
    };

    #[test]
    fn sequential_schema_failure_and_mixed_kind_precedence_match_source() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let group = GroupDescriptor {
            hash: 0,
            labels: OwnedLabels::copy_from(&[], &account, MemoryKind::Workspace),
            whole_drop_name: None,
        };
        let exponential = FloatHistogram {
            count: 1.0,
            sum: 2.0,
            ..FloatHistogram::default()
        };
        let custom = FloatHistogram {
            schema: -53,
            count: 1.0,
            sum: 2.0,
            positive_spans: Some(
                vec![Span {
                    offset: 0,
                    length: 1,
                }]
                .into(),
            ),
            positive_buckets: Some(vec![1.0].into()),
            custom_values: Some(vec![1.0].into()),
            ..FloatHistogram::default()
        };
        let values = [
            OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(exponential.as_ref()),
                &account,
                MemoryKind::Workspace,
            ),
            OwnedValue::copy_from(
                SampleValueRef::FloatHistogram(custom.as_ref()),
                &account,
                MemoryKind::Workspace,
            ),
            OwnedValue::Float(3.0),
        ];
        for operator in [TokenKind::SUM, TokenKind::AVG] {
            for (order, schema, mixed) in [
                ([0, 1, 2], true, false),
                ([0, 2, 1], true, true),
                ([2, 0, 1], false, true),
            ] {
                let mut state = Group::new(&group, 0);
                let notes: Vec<_> = order
                    .into_iter()
                    .filter_map(|index| {
                        state
                            .add(
                                operator,
                                &values[index],
                                true,
                                &Control::default(),
                                &account,
                            )
                            .unwrap()
                    })
                    .collect();
                assert_eq!(
                    notes,
                    if schema {
                        vec![AnnotationKind::MixedExponentialCustomHistograms]
                    } else {
                        vec![]
                    }
                );
                let (result, note) = state
                    .finish(operator, 1000, &Control::default(), &account)
                    .unwrap();
                assert!(result.is_none());
                assert_eq!(
                    note,
                    mixed.then_some(AnnotationKind::MixedFloatsHistogramsAggregation)
                );
            }
        }
        drop(values);
        drop(group);
        account.snapshot().assert_memory_released();
    }

    #[test]
    fn replay_uses_last_emitted_group_flag_before_parent_consumption() {
        let account = EngineAccounting::default().query();
        for whole_flag in [false, true] {
            let descriptor = GroupDescriptor {
                hash: 0,
                labels: OwnedLabels::copy_from(&[], &account, MemoryKind::Workspace),
                whole_drop_name: Some(whole_flag),
            };
            let mut group = Group::new(&descriptor, 0);
            group
                .add(
                    TokenKind::MIN,
                    &OwnedValue::Float(1.0),
                    !whole_flag,
                    &Control::default(),
                    &account,
                )
                .unwrap();
            let (sample, note) = group
                .finish(TokenKind::MIN, 1000, &Control::default(), &account)
                .unwrap();
            assert_eq!(sample.unwrap().drop_name, whole_flag);
            assert!(note.is_none());
        }
        account.snapshot().assert_memory_released();
    }
}
