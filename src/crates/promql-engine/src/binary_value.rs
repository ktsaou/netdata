// SPDX-License-Identifier: GPL-3.0-or-later
// Typed pair outcomes derive from pinned promql/engine.go:3546-3648; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    annotations::{AnnotationKind, HistogramOperation},
    evaluator::{EvaluationError, scalar_binary},
    frontend::token::TokenKind,
    histogram::{GAUGE, HistogramError},
    histogram_work::{HistogramEvaluationError, HistogramWork},
    result::OwnedValue,
    storage::Control,
};

#[derive(Clone, Copy)]
pub(crate) enum Diagnostic {
    Annotation(AnnotationKind),
    // Unrecognized source errors still overwrite the preceding pair's annotation.
    Silent,
}
pub(crate) struct Pair {
    pub(crate) value: Option<OwnedValue>,
    pub(crate) keep: bool,
    pub(crate) diagnostic: Option<Diagnostic>,
}
impl Pair {
    fn valid(value: OwnedValue, keep: bool, diagnostic: Option<Diagnostic>) -> Self {
        Self {
            value: Some(value),
            keep,
            diagnostic,
        }
    }
    fn invalid(diagnostic: Diagnostic) -> Self {
        Self {
            value: None,
            keep: false,
            diagnostic: Some(diagnostic),
        }
    }
}

pub(crate) fn evaluate_scalar(
    operator: TokenKind,
    scalar: f64,
    scalar_left: bool,
    return_bool: bool,
    vector: OwnedValue,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Pair, EvaluationError> {
    let vector_float = match &vector {
        OwnedValue::Float(value) => Some(*value),
        _ => None,
    };
    let scalar = OwnedValue::Float(scalar);
    let (left, right) = if scalar_left {
        (scalar, vector)
    } else {
        (vector, scalar)
    };
    let mut pair = evaluate(operator, left, right, control, account)?;
    if let Some(value) = &mut pair.value {
        if return_bool {
            *value = OwnedValue::Float(f64::from(pair.keep));
        } else if !pair.keep {
            pair.value = None;
        } else if scalar_left && operator.is_comparison() {
            *value = OwnedValue::Float(vector_float.expect("compatible scalar comparison"));
        }
    }
    Ok(pair)
}

pub(crate) fn evaluate(
    operator: TokenKind,
    left: OwnedValue,
    right: OwnedValue,
    control: &Control,
    account: &QueryAccounting,
) -> Result<Pair, EvaluationError> {
    use TokenKind::*;
    control.check()?;
    account.work(1);
    let incompatible = Diagnostic::Annotation(AnnotationKind::IncompatibleBinaryTypes {
        left_histogram: matches!(&left, OwnedValue::Histogram(_)),
        operator,
        right_histogram: matches!(&right, OwnedValue::Histogram(_)),
    });
    Ok(match (left, right) {
        (OwnedValue::Float(left), OwnedValue::Float(right)) => {
            if matches!(operator, TRIM_LOWER | TRIM_UPPER) {
                Pair::invalid(incompatible)
            } else {
                let result = scalar_binary(operator, left, right, control, account)?;
                if operator.is_comparison() {
                    Pair::valid(OwnedValue::Float(left), result != 0.0, None)
                } else {
                    Pair::valid(OwnedValue::Float(result), true, None)
                }
            }
        }
        (OwnedValue::Float(factor), OwnedValue::Histogram(mut histogram)) if operator == MUL => {
            histogram.normalize_copy_fields();
            histogram.mul_checked(factor, control, account)?;
            histogram.compact_checked(control, account)?;
            Pair::valid(OwnedValue::Histogram(histogram), true, None)
        }
        (OwnedValue::Histogram(mut histogram), OwnedValue::Float(value))
            if matches!(operator, MUL | DIV | TRIM_LOWER | TRIM_UPPER) =>
        {
            histogram.normalize_copy_fields();
            match operator {
                MUL => histogram.mul_checked(value, control, account)?,
                DIV => histogram.div_checked(value, control, account)?,
                TRIM_LOWER | TRIM_UPPER => {
                    histogram.trim_checked(value, operator == TRIM_UPPER, control, account)?;
                }
                _ => unreachable!(),
            }
            if matches!(operator, MUL | DIV) {
                histogram.compact_checked(control, account)?;
            }
            Pair::valid(OwnedValue::Histogram(histogram), true, None)
        }
        (OwnedValue::Histogram(mut left), OwnedValue::Histogram(right)) => {
            if matches!(operator, EQLC | NEQ) {
                let equal = left
                    .as_ref()
                    .equals_with(right.as_ref(), HistogramWork::new(control, account))?;
                Pair::valid(
                    OwnedValue::Histogram(left),
                    equal == (operator == EQLC),
                    None,
                )
            } else if matches!(operator, ADD | SUB) {
                left.normalize_copy_fields();
                let result = match left.arithmetic_checked(
                    right.as_ref(),
                    operator == SUB,
                    control,
                    account,
                ) {
                    Ok(result) => result,
                    Err(HistogramEvaluationError::Interrupted(error)) => return Err(error.into()),
                    Err(HistogramEvaluationError::Source(error)) => return Err(error.into()),
                    Err(HistogramEvaluationError::Arithmetic(error)) => {
                        return Ok(Pair::invalid(
                            if error == HistogramError::IncompatibleSchema {
                                Diagnostic::Annotation(AnnotationKind::IncompatibleBinaryLayout(
                                    operator,
                                ))
                            } else {
                                Diagnostic::Silent
                            },
                        ));
                    }
                };
                if operator == SUB {
                    left.set_reset_hint(GAUGE);
                }
                left.compact_checked(control, account)?;
                let operation = if operator == ADD {
                    HistogramOperation::Addition
                } else {
                    HistogramOperation::Subtraction
                };
                if result.counter_reset_collision {
                    Pair::invalid(Diagnostic::Annotation(
                        AnnotationKind::HistogramCounterResetCollision(operation),
                    ))
                } else {
                    Pair::valid(
                        OwnedValue::Histogram(left),
                        true,
                        result
                            .nhcb_bounds_reconciled
                            .then_some(Diagnostic::Annotation(
                                AnnotationKind::MismatchedCustomBuckets(operation),
                            )),
                    )
                }
            } else {
                Pair::invalid(incompatible)
            }
        }
        _ => Pair::invalid(incompatible),
    })
}

pub(crate) fn changes_metadata(operator: TokenKind) -> bool {
    matches!(
        operator,
        TokenKind::ADD
            | TokenKind::SUB
            | TokenKind::MUL
            | TokenKind::DIV
            | TokenKind::POW
            | TokenKind::MOD
            | TokenKind::ATAN2
    )
}
