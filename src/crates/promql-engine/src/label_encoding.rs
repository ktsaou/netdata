// SPDX-License-Identifier: GPL-3.0-or-later
//! Reject the pinned stringlabels encoder's truncated equality boundary.
use crate::{
    accounting::QueryAccounting, binary_labels, evaluator::EvaluationError, result::OwnedLabels,
    storage::Control,
};

pub(crate) const FIELD_BOUNDARY: usize = 1 << 24;
pub(crate) const TOO_LONG: &str = "String too long to encode as label.";

pub(crate) fn fits(bytes: &[u8]) -> bool {
    bytes.len() < FIELD_BOUNDARY
}

pub(crate) fn field(
    bytes: &[u8],
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    control.check()?;
    account.work(1);
    if !fits(bytes) {
        return Err(EvaluationError::Runtime(binary_labels::formatted(
            format_args!("{TOO_LONG}"),
            control,
            account,
        )?));
    }
    Ok(())
}

pub(crate) fn finalized(
    labels: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<(), EvaluationError> {
    for label in &*labels.0 {
        field(&label.name, control, account)?;
        field(&label.value, control, account)?;
    }
    Ok(())
}
