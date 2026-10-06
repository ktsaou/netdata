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
    numerical::{compensated_add, source_add, sqrt},
    result::{OwnedLabels, OwnedValue},
    storage::{Control, Interrupted},
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
        account: &QueryAccounting,
    ) -> Option<AnnotationKind> {
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
                            NativeAccumulator::new(operator == AVG, histogram.copy()),
                            &AllocationContext::new(account, MemoryKind::Workspace),
                        ));
                        self.has_histogram = true;
                    } else {
                        self.has_float = true;
                    }
                }
                MIN | MAX | STDVAR | STDDEV if histogram.is_some() => {
                    self.seen = false;
                    return Some(AnnotationKind::HistogramIgnoredInAggregation(operator));
                }
                STDVAR | STDDEV => {
                    self.value = if float.is_nan() || float.is_infinite() {
                        NAN
                    } else {
                        0.0
                    };
                }
                GROUP => self.value = 1.0,
                _ => (),
            }
            return None;
        }
        // Upstream stops a schema-invalid group before consuming later flags or sample kinds.
        if self.incompatible {
            return None;
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
                        return match native.add(histogram.as_ref(), self.count, account) {
                            Ok(true) => Some(AnnotationKind::MismatchedCustomBuckets(
                                HistogramOperation::Aggregation,
                            )),
                            Ok(false) => None,
                            Err(HistogramError::IncompatibleSchema) => {
                                self.incompatible = true;
                                Some(AnnotationKind::MixedExponentialCustomHistograms)
                            }
                            Err(error) => panic!("{error}"),
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
                                return None;
                            }
                            self.incremental_mean = true;
                            self.mean = self.value / (self.count - 1.0);
                            self.compensation /= self.count - 1.0;
                        }
                        let q = (self.count - 1.0) / self.count;
                        (self.mean, self.compensation) = compensated_add(
                            float / self.count,
                            q * self.mean,
                            q * self.compensation,
                        );
                    }
                }
            }
            COUNT => self.count += 1.0,
            MIN | MAX | STDVAR | STDDEV if histogram.is_some() => {
                return Some(AnnotationKind::HistogramIgnoredInAggregation(operator));
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
                let delta = float - self.mean;
                self.mean += delta / self.count;
                self.value += delta * (float - self.mean);
            }
            GROUP => (),
            _ => unreachable!("qualified aggregation operator"),
        }
        None
    }
    pub(crate) fn finish(
        mut self,
        operator: TokenKind,
        time_ms: i64,
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(Option<EvaluatedSample>, Option<AnnotationKind>), Interrupted> {
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
            let mut result = native
                .finish(self.count, Finalization::Vector)
                .expect("vector final correction");
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
                AVG => source_add(self.value / self.count, self.compensation / self.count),
                COUNT => self.count,
                GROUP => 1.0,
                STDVAR => self.value / self.count,
                STDDEV => sqrt(self.value / self.count),
                _ => self.value,
            };
            (OwnedValue::Float(value), None)
        };
        let mut labels =
            OwnedLabels::copy_from(&self.labels.views(account), account, MemoryKind::Workspace);
        // Source removes empty label values only after hashing the raw grouping key.
        labels.retain_labels(|label| !label.value.is_empty());
        Ok((
            Some(EvaluatedSample {
                ordinal: self.ordinal,
                labels,
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
                    .filter_map(|index| state.add(operator, &values[index], true, &account))
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
            group.add(
                TokenKind::MIN,
                &OwnedValue::Float(1.0),
                !whole_flag,
                &account,
            );
            let (sample, note) = group
                .finish(TokenKind::MIN, 1000, &Control::default(), &account)
                .unwrap();
            assert_eq!(sample.unwrap().drop_name, whole_flag);
            assert!(note.is_none());
        }
        account.snapshot().assert_memory_released();
    }
}
