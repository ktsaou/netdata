// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{accounting::AccountedVec, storage::Interrupted};
use std::ops::Deref;

use super::work::Work;

/// Tokens borrow input; decoded literals and diagnostics keep their allocation owner.
#[derive(Clone, Debug)]
pub enum Bytes<'a> {
    Borrowed(&'a [u8]),
    Owned(AccountedVec<u8>),
}
impl Bytes<'_> {
    pub(super) fn copy_in(&self, work: Work<'_>) -> Result<Self, Interrupted> {
        Ok(match self {
            Self::Borrowed(bytes) => Self::Borrowed(bytes),
            Self::Owned(bytes) => Self::Owned(work.copy(bytes)?),
        })
    }
    pub(super) fn into_owned(self, work: Work<'_>) -> Result<AccountedVec<u8>, Interrupted> {
        match self {
            Self::Borrowed(bytes) => work.copy(bytes),
            Self::Owned(bytes) => Ok(bytes),
        }
    }
}
impl Deref for Bytes<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Borrowed(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}
impl AsRef<[u8]> for Bytes<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl PartialEq for Bytes<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl Eq for Bytes<'_> {}
