// SPDX-License-Identifier: GPL-3.0-or-later
//! Shape checks precede raw views; pointer liveness remains the embedding caller's contract.
use super::types::{self, Bytes};
use crate::{
    accounting::{AccountedVec, MemoryKind, QueryAccounting},
    storage::Control,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Invalid(pub &'static str);

pub(crate) fn region<T>(data: *const T, len: u64) -> Result<(usize, usize), Invalid> {
    let len =
        usize::try_from(len).map_err(|_| Invalid("array count does not fit address space"))?;
    let bytes = len
        .checked_mul(size_of::<T>())
        .filter(|bytes| *bytes <= isize::MAX as usize)
        .ok_or(Invalid("array byte size overflow"))?;
    if bytes == 0 {
        return Ok((0, 0));
    }
    let start = data as usize;
    if data.is_null() || !start.is_multiple_of(align_of::<T>()) {
        return Err(Invalid("null or misaligned pointer"));
    }
    let end = start
        .checked_add(bytes)
        .ok_or(Invalid("array address overflow"))?;
    Ok((start, end))
}
pub(crate) unsafe fn slice<'a, T>(data: *const T, len: u64) -> Result<&'a [T], Invalid> {
    let (start, _) = region(data, len)?;
    if start == 0 {
        return Ok(&[]);
    }
    // A live, initialized, immutable single-allocation region is a caller precondition.
    Ok(unsafe { std::slice::from_raw_parts(data, len as usize) })
}
pub(crate) unsafe fn bytes<'a>(value: Bytes) -> Result<&'a [u8], Invalid> {
    unsafe { slice(value.data, value.len) }
}
pub(crate) unsafe fn read<T: Copy>(value: *const T) -> Result<T, Invalid> {
    region(value, 1)?;
    Ok(unsafe { value.read() })
}
pub(crate) unsafe fn versioned<T: Copy>(value: *const T) -> Result<T, Invalid> {
    if value.is_null() || !(value as usize).is_multiple_of(align_of::<T>()) {
        return Err(Invalid("null or misaligned ABI structure"));
    }
    // Reading a full T before checking this prefix would already require the missing bytes.
    let header = unsafe { read(value.cast::<types::Header>()) }?;
    if header.abi_version != types::ABI_VERSION || (header.struct_size as usize) < size_of::<T>() {
        return Err(Invalid("unsupported ABI version or short structure"));
    }
    unsafe { read(value) }
}
pub(crate) unsafe fn output<T>(value: *mut T) -> Result<(), Invalid> {
    region(value, 1).map(|_| ())
}
pub(crate) fn distinct_outputs<A, B>(a: *mut A, b: *mut B) -> Result<(), Invalid> {
    let (a, ae) = region(a, 1)?;
    let (b, be) = region(b, 1)?;
    if a < be && b < ae {
        return Err(Invalid("overlapping output slots"));
    }
    Ok(())
}
pub(crate) fn present(value: u32) -> Result<bool, Invalid> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Invalid("invalid presence bit")),
    }
}
pub(crate) unsafe fn optional<'a, T>(
    data: *const T,
    len: u64,
    presence: u32,
    reserved: u32,
) -> Result<Option<&'a [T]>, Invalid> {
    if reserved != 0 {
        return Err(Invalid("nonzero reserved array field"));
    }
    if !present(presence)? {
        if len != 0 {
            return Err(Invalid("absent array has a nonzero count"));
        }
        return Ok(None);
    }
    Ok(Some(unsafe { slice(data, len) }?))
}
pub(crate) fn callback_control(control: &Control) -> *const types::CallbackControl {
    (control as *const Control).cast()
}
pub(crate) unsafe fn returned_error(
    value: Bytes,
    account: &QueryAccounting,
) -> Result<AccountedVec<u8>, Invalid> {
    let value = unsafe { bytes(value) }?;
    let mut result = AccountedVec::new(account, MemoryKind::Workspace);
    // A returned callback error wins over concurrent cancellation, including while its bytes are retained.
    for chunk in value.chunks(1024) {
        account.work(chunk.len() as u64);
        result.extend_from_slice(chunk);
    }
    Ok(result)
}

#[derive(Clone, Copy)]
struct Range {
    start: usize,
    end: usize,
}
pub(crate) struct Footprint {
    ranges: AccountedVec<Range>,
}
impl Footprint {
    pub(crate) fn new(account: &QueryAccounting) -> Self {
        Self {
            ranges: AccountedVec::new(account, MemoryKind::Workspace),
        }
    }
    pub(crate) fn add<T>(&mut self, data: *const T, len: u64) -> Result<(), Invalid> {
        let (start, end) = region(data, len)?;
        if end != 0 {
            self.ranges.push(Range { start, end });
        }
        Ok(())
    }
    pub(crate) fn add_slice<T>(&mut self, value: Option<&[T]>) -> Result<(), Invalid> {
        if let Some(value) = value {
            self.add(value.as_ptr(), value.len() as u64)?;
        }
        Ok(())
    }
    pub(crate) fn bytes(mut self) -> Result<u64, Invalid> {
        // Match native page exposure: observe the complete bounded page before cooperative conversion.
        self.ranges.sort_unstable_by_key(|range| range.start);
        let mut bytes = 0u64;
        let mut end = 0;
        for range in &*self.ranges {
            if range.end > end {
                bytes = bytes
                    .checked_add((range.end - range.start.max(end)) as u64)
                    .ok_or(Invalid("page exposure overflow"))?;
                end = range.end;
            }
        }
        Ok(bytes)
    }
}
