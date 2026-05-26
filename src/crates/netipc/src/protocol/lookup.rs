//! CGROUPS_LOOKUP and APPS_LOOKUP codecs.

use super::cgroups::StrView;
use super::{
    align8, NipcError, ALIGNMENT, APPS_CGROUP_HOST_ROOT, APPS_CGROUP_KNOWN,
    APPS_CGROUP_UNKNOWN_PERMANENT, APPS_CGROUP_UNKNOWN_RETRY_LATER, CGROUP_LOOKUP_KNOWN,
    CGROUP_LOOKUP_UNKNOWN_PERMANENT, CGROUP_LOOKUP_UNKNOWN_RETRY_LATER, NIPC_UID_UNSET,
    PID_LOOKUP_KNOWN, PID_LOOKUP_UNKNOWN,
};

pub const CGROUPS_LOOKUP_REQ_HDR_SIZE: usize = 16;
pub const CGROUPS_LOOKUP_RESP_HDR_SIZE: usize = 16;
pub const CGROUPS_LOOKUP_ITEM_HDR_SIZE: usize = 28;
pub const APPS_LOOKUP_REQ_HDR_SIZE: usize = 16;
pub const APPS_LOOKUP_RESP_HDR_SIZE: usize = 16;
pub const APPS_LOOKUP_ITEM_HDR_SIZE: usize = 60;
pub const LOOKUP_DIR_ENTRY_SIZE: usize = 8;
pub const LOOKUP_LABEL_ENTRY_SIZE: usize = 16;
pub const APPS_LOOKUP_KEY_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookupLabelView<'a> {
    pub key: StrView<'a>,
    pub value: StrView<'a>,
}

#[derive(Debug)]
pub struct CgroupsLookupRequestView<'a> {
    pub item_count: u32,
    payload: &'a [u8],
}

#[derive(Debug)]
pub struct AppsLookupRequestView<'a> {
    pub item_count: u32,
    payload: &'a [u8],
}

#[derive(Debug)]
pub struct CgroupsLookupResponseView<'a> {
    pub layout_version: u16,
    pub flags: u16,
    pub item_count: u32,
    pub generation: u64,
    payload: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CgroupsLookupItemView<'a> {
    pub status: u16,
    pub orchestrator: u16,
    pub path: StrView<'a>,
    pub name: StrView<'a>,
    pub label_count: u16,
    item: &'a [u8],
    label_table_offset: usize,
}

#[derive(Debug)]
pub struct AppsLookupResponseView<'a> {
    pub layout_version: u16,
    pub flags: u16,
    pub item_count: u32,
    pub generation: u64,
    payload: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppsLookupItemView<'a> {
    pub status: u16,
    pub orchestrator: u16,
    pub cgroup_status: u16,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub starttime: u64,
    pub comm: StrView<'a>,
    pub cgroup_path: StrView<'a>,
    pub cgroup_name: StrView<'a>,
    pub label_count: u16,
    item: &'a [u8],
    label_table_offset: usize,
}

#[inline]
fn u16_at(buf: &[u8], off: usize) -> u16 {
    u16::from_ne_bytes(buf[off..off + 2].try_into().unwrap())
}

#[inline]
fn u32_at(buf: &[u8], off: usize) -> u32 {
    u32::from_ne_bytes(buf[off..off + 4].try_into().unwrap())
}

#[inline]
fn u64_at(buf: &[u8], off: usize) -> u64 {
    u64::from_ne_bytes(buf[off..off + 8].try_into().unwrap())
}

#[inline]
fn put_u16(buf: &mut [u8], off: usize, value: u16) {
    buf[off..off + 2].copy_from_slice(&value.to_ne_bytes());
}

#[inline]
fn put_u32(buf: &mut [u8], off: usize, value: u32) {
    buf[off..off + 4].copy_from_slice(&value.to_ne_bytes());
}

#[inline]
fn put_u64(buf: &mut [u8], off: usize, value: u64) {
    buf[off..off + 8].copy_from_slice(&value.to_ne_bytes());
}

fn checked_u32(value: usize) -> Result<u32, NipcError> {
    u32::try_from(value).map_err(|_| NipcError::Overflow)
}

fn checked_u16(value: usize) -> Result<u16, NipcError> {
    u16::try_from(value).map_err(|_| NipcError::Overflow)
}

fn source_string_invalid(bytes: &[u8], require_non_empty: bool) -> bool {
    (require_non_empty && bytes.is_empty()) || bytes.contains(&0)
}

fn validate_lookup_dir(
    buf: &[u8],
    dir_start: usize,
    item_count: u32,
    packed_area_len: usize,
    min_len: usize,
    exact_len: Option<usize>,
) -> Result<(), NipcError> {
    let dir_size = (item_count as usize)
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .ok_or(NipcError::BadItemCount)?;
    if dir_start
        .checked_add(dir_size)
        .ok_or(NipcError::BadItemCount)?
        > buf.len()
    {
        return Err(NipcError::Truncated);
    }

    let mut prev_end = 0usize;
    for i in 0..item_count as usize {
        let base = dir_start + i * LOOKUP_DIR_ENTRY_SIZE;
        let off = u32_at(buf, base) as usize;
        let len = u32_at(buf, base + 4) as usize;
        if off % ALIGNMENT != 0 {
            return Err(NipcError::BadAlignment);
        }
        if let Some(exact) = exact_len {
            if len != exact {
                return Err(NipcError::BadLayout);
            }
        } else if len < min_len {
            return Err(NipcError::BadLayout);
        }
        let end = off.checked_add(len).ok_or(NipcError::OutOfBounds)?;
        if end > packed_area_len {
            return Err(NipcError::OutOfBounds);
        }
        if i > 0 && off < prev_end {
            return Err(NipcError::BadLayout);
        }
        prev_end = end;
    }
    Ok(())
}

fn lookup_string<'a>(
    item: &'a [u8],
    hdr_size: usize,
    offset: usize,
    length: usize,
) -> Result<(StrView<'a>, usize), NipcError> {
    if offset < hdr_size {
        return Err(NipcError::OutOfBounds);
    }
    let nul = offset.checked_add(length).ok_or(NipcError::OutOfBounds)?;
    if nul >= item.len() {
        return Err(NipcError::OutOfBounds);
    }
    if item[nul] != 0 {
        return Err(NipcError::MissingNul);
    }
    if item[offset..nul].contains(&0) {
        return Err(NipcError::BadLayout);
    }
    Ok((
        StrView {
            bytes: &item[offset..nul + 1],
            len: length as u32,
        },
        nul + 1,
    ))
}

#[inline]
fn overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

fn checked_subslice<'a>(
    buf: &'a [u8],
    base: usize,
    offset: usize,
    len: usize,
) -> Result<&'a [u8], NipcError> {
    let start = base.checked_add(offset).ok_or(NipcError::OutOfBounds)?;
    let end = start.checked_add(len).ok_or(NipcError::OutOfBounds)?;
    buf.get(start..end).ok_or(NipcError::OutOfBounds)
}

fn lookup_data_offset(hdr_size: usize, item_count: u32) -> Result<usize, NipcError> {
    (item_count as usize)
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .and_then(|v| hdr_size.checked_add(v))
        .ok_or(NipcError::BadItemCount)
}

fn lookup_dir_entry_offset(hdr_size: usize, index: u32) -> Result<usize, NipcError> {
    (index as usize)
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .and_then(|v| hdr_size.checked_add(v))
        .ok_or(NipcError::BadItemCount)
}

fn validate_labels(
    item: &[u8],
    hdr_size: usize,
    label_count: u16,
    fixed_end: usize,
) -> Result<usize, NipcError> {
    if label_count == 0 {
        if fixed_end != item.len() {
            return Err(NipcError::BadLayout);
        }
        return Ok(fixed_end);
    }

    let table_start = align8(fixed_end);
    if table_start > item.len() {
        return Err(NipcError::OutOfBounds);
    }
    if item[fixed_end..table_start].iter().any(|&b| b != 0) {
        return Err(NipcError::BadLayout);
    }

    let table_bytes = (label_count as usize)
        .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
        .ok_or(NipcError::OutOfBounds)?;
    let mut expected = table_start
        .checked_add(table_bytes)
        .ok_or(NipcError::OutOfBounds)?;
    if expected > item.len() {
        return Err(NipcError::OutOfBounds);
    }

    for i in 0..label_count as usize {
        let entry_rel = i
            .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
            .ok_or(NipcError::OutOfBounds)?;
        let base = table_start
            .checked_add(entry_rel)
            .ok_or(NipcError::OutOfBounds)?;
        let key_off = u32_at(item, base) as usize;
        let key_len = u32_at(item, base + 4) as usize;
        let value_off = u32_at(item, base + 8) as usize;
        let value_len = u32_at(item, base + 12) as usize;
        if key_len == 0 || key_off != expected {
            return Err(NipcError::BadLayout);
        }
        let (_, key_end) = lookup_string(item, hdr_size, key_off, key_len)?;
        expected = key_end;
        if value_off != expected {
            return Err(NipcError::BadLayout);
        }
        let (_, value_end) = lookup_string(item, hdr_size, value_off, value_len)?;
        expected = value_end;
    }

    if expected != item.len() {
        return Err(NipcError::BadLayout);
    }
    Ok(table_start)
}

fn label_at<'a>(
    item: &'a [u8],
    hdr_size: usize,
    label_count: u16,
    label_table_offset: usize,
    index: u32,
) -> Result<LookupLabelView<'a>, NipcError> {
    if index >= label_count as u32 {
        return Err(NipcError::OutOfBounds);
    }
    let entry_offset = (index as usize)
        .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
        .ok_or(NipcError::OutOfBounds)?;
    let base = label_table_offset
        .checked_add(entry_offset)
        .ok_or(NipcError::OutOfBounds)?;
    let entry_end = base
        .checked_add(LOOKUP_LABEL_ENTRY_SIZE)
        .ok_or(NipcError::OutOfBounds)?;
    if entry_end > item.len() {
        return Err(NipcError::OutOfBounds);
    }
    let key_off = u32_at(item, base) as usize;
    let key_len = u32_at(item, base + 4) as usize;
    let value_off = u32_at(item, base + 8) as usize;
    let value_len = u32_at(item, base + 12) as usize;
    let (key, _) = lookup_string(item, hdr_size, key_off, key_len)?;
    let (value, _) = lookup_string(item, hdr_size, value_off, value_len)?;
    Ok(LookupLabelView { key, value })
}

pub fn encode_cgroups_lookup_request(paths: &[&[u8]], buf: &mut [u8]) -> Result<usize, NipcError> {
    let count = paths.len();
    let dir_size = count
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .ok_or(NipcError::Overflow)?;
    let packed_start = CGROUPS_LOOKUP_REQ_HDR_SIZE
        .checked_add(dir_size)
        .ok_or(NipcError::Overflow)?;
    if buf.len() < packed_start {
        return Err(NipcError::Overflow);
    }

    let mut data = packed_start;
    for (i, path) in paths.iter().enumerate() {
        if source_string_invalid(path, true) {
            return Err(NipcError::BadLayout);
        }
        let aligned = align8(data);
        let key_len = path.len().checked_add(1).ok_or(NipcError::Overflow)?;
        let end = aligned.checked_add(key_len).ok_or(NipcError::Overflow)?;
        if end > buf.len() {
            return Err(NipcError::Overflow);
        }
        if aligned > data {
            buf[data..aligned].fill(0);
        }

        let dir = CGROUPS_LOOKUP_REQ_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
        put_u32(buf, dir, checked_u32(aligned - packed_start)?);
        put_u32(buf, dir + 4, checked_u32(key_len)?);
        buf[aligned..aligned + path.len()].copy_from_slice(path);
        buf[aligned + path.len()] = 0;
        data = end;
    }

    put_u16(buf, 0, 1);
    put_u16(buf, 2, 0);
    put_u32(buf, 4, checked_u32(count)?);
    put_u32(buf, 8, 0);
    put_u32(buf, 12, 0);
    Ok(data)
}

impl<'a> CgroupsLookupRequestView<'a> {
    pub fn decode(buf: &'a [u8]) -> Result<Self, NipcError> {
        if buf.len() < CGROUPS_LOOKUP_REQ_HDR_SIZE {
            return Err(NipcError::Truncated);
        }
        if u16_at(buf, 0) != 1 || u16_at(buf, 2) != 0 || u32_at(buf, 8) != 0 || u32_at(buf, 12) != 0
        {
            return Err(NipcError::BadLayout);
        }
        let item_count = u32_at(buf, 4);
        let dir_size = (item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let dir_end = CGROUPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        if dir_end > buf.len() {
            return Err(NipcError::Truncated);
        }
        let packed_len = buf.len() - dir_end;
        validate_lookup_dir(
            buf,
            CGROUPS_LOOKUP_REQ_HDR_SIZE,
            item_count,
            packed_len,
            2,
            None,
        )?;
        for i in 0..item_count as usize {
            let base = CGROUPS_LOOKUP_REQ_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
            let off = u32_at(buf, base) as usize;
            let len = u32_at(buf, base + 4) as usize;
            let key = checked_subslice(buf, dir_end, off, len)?;
            if key[len - 1] != 0 {
                return Err(NipcError::MissingNul);
            }
            if key[..len - 1].contains(&0) {
                return Err(NipcError::BadLayout);
            }
        }
        Ok(Self {
            item_count,
            payload: buf,
        })
    }

    pub fn item(&self, index: u32) -> Result<StrView<'a>, NipcError> {
        if index >= self.item_count {
            return Err(NipcError::OutOfBounds);
        }
        let dir_size = (self.item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let packed_start = CGROUPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        let base = CGROUPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(
                (index as usize)
                    .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
                    .ok_or(NipcError::BadItemCount)?,
            )
            .ok_or(NipcError::BadItemCount)?;
        let off = u32_at(self.payload, base) as usize;
        let len = u32_at(self.payload, base + 4) as usize;
        Ok(StrView {
            bytes: checked_subslice(self.payload, packed_start, off, len)?,
            len: (len - 1) as u32,
        })
    }
}

pub fn encode_apps_lookup_request(pids: &[u32], buf: &mut [u8]) -> Result<usize, NipcError> {
    let count = pids.len();
    let dir_size = count
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .ok_or(NipcError::Overflow)?;
    let key_size = count
        .checked_mul(APPS_LOOKUP_KEY_SIZE)
        .ok_or(NipcError::Overflow)?;
    let packed_start = APPS_LOOKUP_REQ_HDR_SIZE
        .checked_add(dir_size)
        .ok_or(NipcError::Overflow)?;
    let total = packed_start
        .checked_add(key_size)
        .ok_or(NipcError::Overflow)?;
    if total > buf.len() {
        return Err(NipcError::Overflow);
    }

    for (i, pid) in pids.iter().enumerate() {
        let dir = APPS_LOOKUP_REQ_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
        let key_offset = i
            .checked_mul(APPS_LOOKUP_KEY_SIZE)
            .ok_or(NipcError::Overflow)?;
        put_u32(buf, dir, checked_u32(key_offset)?);
        put_u32(buf, dir + 4, APPS_LOOKUP_KEY_SIZE as u32);
        let key = packed_start + i * APPS_LOOKUP_KEY_SIZE;
        put_u32(buf, key, *pid);
        put_u32(buf, key + 4, 0);
    }

    put_u16(buf, 0, 1);
    put_u16(buf, 2, 0);
    put_u32(buf, 4, checked_u32(count)?);
    put_u32(buf, 8, 0);
    put_u32(buf, 12, 0);
    Ok(total)
}

impl<'a> AppsLookupRequestView<'a> {
    pub fn decode(buf: &'a [u8]) -> Result<Self, NipcError> {
        if buf.len() < APPS_LOOKUP_REQ_HDR_SIZE {
            return Err(NipcError::Truncated);
        }
        if u16_at(buf, 0) != 1 || u16_at(buf, 2) != 0 || u32_at(buf, 8) != 0 || u32_at(buf, 12) != 0
        {
            return Err(NipcError::BadLayout);
        }
        let item_count = u32_at(buf, 4);
        let dir_size = (item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let dir_end = APPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        if dir_end > buf.len() {
            return Err(NipcError::Truncated);
        }
        validate_lookup_dir(
            buf,
            APPS_LOOKUP_REQ_HDR_SIZE,
            item_count,
            buf.len() - dir_end,
            0,
            Some(APPS_LOOKUP_KEY_SIZE),
        )?;
        for i in 0..item_count as usize {
            let base = APPS_LOOKUP_REQ_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
            let off = u32_at(buf, base) as usize;
            let key = checked_subslice(buf, dir_end, off, APPS_LOOKUP_KEY_SIZE)?;
            if u32_at(key, 4) != 0 {
                return Err(NipcError::BadLayout);
            }
        }
        Ok(Self {
            item_count,
            payload: buf,
        })
    }

    pub fn item(&self, index: u32) -> Result<u32, NipcError> {
        if index >= self.item_count {
            return Err(NipcError::OutOfBounds);
        }
        let dir_size = (self.item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let packed_start = APPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        let base = APPS_LOOKUP_REQ_HDR_SIZE
            .checked_add(
                (index as usize)
                    .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
                    .ok_or(NipcError::BadItemCount)?,
            )
            .ok_or(NipcError::BadItemCount)?;
        let off = u32_at(self.payload, base) as usize;
        Ok(u32_at(
            checked_subslice(self.payload, packed_start, off, APPS_LOOKUP_KEY_SIZE)?,
            0,
        ))
    }
}

impl<'a> CgroupsLookupResponseView<'a> {
    pub fn decode(buf: &'a [u8]) -> Result<Self, NipcError> {
        if buf.len() < CGROUPS_LOOKUP_RESP_HDR_SIZE {
            return Err(NipcError::Truncated);
        }
        if u16_at(buf, 0) != 1 || u16_at(buf, 2) != 0 {
            return Err(NipcError::BadLayout);
        }
        let item_count = u32_at(buf, 4);
        let dir_size = (item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let dir_end = CGROUPS_LOOKUP_RESP_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        if dir_end > buf.len() {
            return Err(NipcError::Truncated);
        }
        validate_lookup_dir(
            buf,
            CGROUPS_LOOKUP_RESP_HDR_SIZE,
            item_count,
            buf.len() - dir_end,
            CGROUPS_LOOKUP_ITEM_HDR_SIZE,
            None,
        )?;
        for i in 0..item_count as usize {
            let base = CGROUPS_LOOKUP_RESP_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
            let off = u32_at(buf, base) as usize;
            let len = u32_at(buf, base + 4) as usize;
            decode_cgroups_item(checked_subslice(buf, dir_end, off, len)?)?;
        }
        Ok(Self {
            layout_version: 1,
            flags: 0,
            item_count,
            generation: u64_at(buf, 8),
            payload: buf,
        })
    }

    pub fn item(&self, index: u32) -> Result<CgroupsLookupItemView<'a>, NipcError> {
        if index >= self.item_count {
            return Err(NipcError::OutOfBounds);
        }
        let packed_start = lookup_data_offset(CGROUPS_LOOKUP_RESP_HDR_SIZE, self.item_count)?;
        let base = lookup_dir_entry_offset(CGROUPS_LOOKUP_RESP_HDR_SIZE, index)?;
        let off = u32_at(self.payload, base) as usize;
        let len = u32_at(self.payload, base + 4) as usize;
        decode_cgroups_item(checked_subslice(self.payload, packed_start, off, len)?)
    }
}

fn decode_cgroups_item(item: &[u8]) -> Result<CgroupsLookupItemView<'_>, NipcError> {
    if item.len() < CGROUPS_LOOKUP_ITEM_HDR_SIZE {
        return Err(NipcError::Truncated);
    }
    let status = u16_at(item, 2);
    let orchestrator = u16_at(item, 4);
    let path_off = u32_at(item, 8) as usize;
    let path_len = u32_at(item, 12) as usize;
    let name_off = u32_at(item, 16) as usize;
    let name_len = u32_at(item, 20) as usize;
    let label_count = u16_at(item, 24);

    if u16_at(item, 0) != 1 || u16_at(item, 6) != 0 || u16_at(item, 26) != 0 {
        return Err(NipcError::BadLayout);
    }
    if status != CGROUP_LOOKUP_KNOWN
        && status != CGROUP_LOOKUP_UNKNOWN_RETRY_LATER
        && status != CGROUP_LOOKUP_UNKNOWN_PERMANENT
    {
        return Err(NipcError::BadLayout);
    }
    if path_len == 0 {
        return Err(NipcError::BadLayout);
    }
    if status != CGROUP_LOOKUP_KNOWN && (orchestrator != 0 || name_len != 0 || label_count != 0) {
        return Err(NipcError::BadLayout);
    }

    let (path, path_end) = lookup_string(item, CGROUPS_LOOKUP_ITEM_HDR_SIZE, path_off, path_len)?;
    let (name, name_end) = lookup_string(item, CGROUPS_LOOKUP_ITEM_HDR_SIZE, name_off, name_len)?;
    if overlap(path_off, path_end, name_off, name_end) {
        return Err(NipcError::BadLayout);
    }
    let label_table_offset = validate_labels(
        item,
        CGROUPS_LOOKUP_ITEM_HDR_SIZE,
        label_count,
        path_end.max(name_end),
    )?;
    Ok(CgroupsLookupItemView {
        status,
        orchestrator,
        path,
        name,
        label_count,
        item,
        label_table_offset,
    })
}

impl<'a> CgroupsLookupItemView<'a> {
    pub fn label(&self, index: u32) -> Result<LookupLabelView<'a>, NipcError> {
        label_at(
            self.item,
            CGROUPS_LOOKUP_ITEM_HDR_SIZE,
            self.label_count,
            self.label_table_offset,
            index,
        )
    }
}

pub struct CgroupsLookupBuilder<'a> {
    buf: &'a mut [u8],
    generation: u64,
    item_count: u32,
    max_items: u32,
    data_offset: usize,
    error: Option<NipcError>,
}

impl<'a> CgroupsLookupBuilder<'a> {
    pub fn new(buf: &'a mut [u8], max_items: u32, generation: u64) -> Self {
        let data_offset = (max_items as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .and_then(|v| CGROUPS_LOOKUP_RESP_HDR_SIZE.checked_add(v))
            .expect("CgroupsLookupBuilder buffer too small");
        assert!(
            buf.len() >= data_offset,
            "CgroupsLookupBuilder buffer too small"
        );
        Self {
            buf,
            generation,
            item_count: 0,
            max_items,
            data_offset,
            error: None,
        }
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    pub fn add(
        &mut self,
        status: u16,
        orchestrator: u16,
        path: &[u8],
        name: &[u8],
        labels: &[(&[u8], &[u8])],
    ) -> Result<(), NipcError> {
        if self.item_count >= self.max_items {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        }
        if status != CGROUP_LOOKUP_KNOWN
            && status != CGROUP_LOOKUP_UNKNOWN_RETRY_LATER
            && status != CGROUP_LOOKUP_UNKNOWN_PERMANENT
        {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        if source_string_invalid(path, true) || source_string_invalid(name, false) {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        if status != CGROUP_LOOKUP_KNOWN
            && (orchestrator != 0 || !name.is_empty() || !labels.is_empty())
        {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        let label_count = match checked_u16(labels.len()) {
            Ok(v) => v,
            Err(err) => {
                self.error = Some(err);
                return Err(err);
            }
        };

        let item_start = align8(self.data_offset);
        let path_offset = CGROUPS_LOOKUP_ITEM_HDR_SIZE;
        let Some(name_offset) = path_offset
            .checked_add(path.len())
            .and_then(|v| v.checked_add(1))
        else {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        };
        let Some(fixed_end) = name_offset
            .checked_add(name.len())
            .and_then(|v| v.checked_add(1))
        else {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        };
        let (table_start, table_bytes, mut item_size) = label_layout(fixed_end, labels)?;
        let item_end = item_start
            .checked_add(item_size)
            .ok_or(NipcError::Overflow)?;
        if item_end > self.buf.len() {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        }
        if item_start > self.data_offset {
            self.buf[self.data_offset..item_start].fill(0);
        }
        let item = &mut self.buf[item_start..item_end];
        if let Err(err) = write_cgroups_item_header(
            item,
            status,
            orchestrator,
            path_offset,
            path.len(),
            name_offset,
            name.len(),
            label_count,
        ) {
            self.error = Some(err);
            return Err(err);
        }
        item[path_offset..path_offset + path.len()].copy_from_slice(path);
        item[path_offset + path.len()] = 0;
        item[name_offset..name_offset + name.len()].copy_from_slice(name);
        item[name_offset + name.len()] = 0;
        if !labels.is_empty() {
            item[fixed_end..table_start].fill(0);
            let mut next = table_start
                .checked_add(table_bytes)
                .ok_or(NipcError::Overflow)?;
            for (i, (key, value)) in labels.iter().enumerate() {
                if source_string_invalid(key, true) || source_string_invalid(value, false) {
                    self.error = Some(NipcError::BadLayout);
                    return Err(NipcError::BadLayout);
                }
                let entry_offset = i
                    .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
                    .ok_or(NipcError::Overflow)?;
                let entry = table_start
                    .checked_add(entry_offset)
                    .ok_or(NipcError::Overflow)?;
                let value_offset = next
                    .checked_add(key.len())
                    .and_then(|v| v.checked_add(1))
                    .ok_or(NipcError::Overflow)?;
                put_u32(item, entry, checked_u32(next)?);
                put_u32(item, entry + 4, checked_u32(key.len())?);
                put_u32(item, entry + 8, checked_u32(value_offset)?);
                put_u32(item, entry + 12, checked_u32(value.len())?);
                item[next..next + key.len()].copy_from_slice(key);
                item[next + key.len()] = 0;
                next = value_offset;
                item[next..next + value.len()].copy_from_slice(value);
                item[next + value.len()] = 0;
                next = next
                    .checked_add(value.len())
                    .and_then(|v| v.checked_add(1))
                    .ok_or(NipcError::Overflow)?;
            }
            item_size = next;
        }
        let dir_offset = (self.item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::Overflow)?;
        let dir = CGROUPS_LOOKUP_RESP_HDR_SIZE
            .checked_add(dir_offset)
            .ok_or(NipcError::Overflow)?;
        put_u32(self.buf, dir, checked_u32(item_start)?);
        put_u32(self.buf, dir + 4, checked_u32(item_size)?);
        self.data_offset = item_end;
        self.item_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<usize, NipcError> {
        finish_lookup_response(
            self.buf,
            CGROUPS_LOOKUP_RESP_HDR_SIZE,
            self.item_count,
            self.data_offset,
            self.generation,
        )
    }

    pub fn error(&self) -> Option<NipcError> {
        self.error
    }

    pub fn item_count(&self) -> u32 {
        self.item_count
    }
}

fn write_cgroups_item_header(
    item: &mut [u8],
    status: u16,
    orchestrator: u16,
    path_off: usize,
    path_len: usize,
    name_off: usize,
    name_len: usize,
    label_count: u16,
) -> Result<(), NipcError> {
    put_u16(item, 0, 1);
    put_u16(item, 2, status);
    put_u16(item, 4, orchestrator);
    put_u16(item, 6, 0);
    put_u32(item, 8, checked_u32(path_off)?);
    put_u32(item, 12, checked_u32(path_len)?);
    put_u32(item, 16, checked_u32(name_off)?);
    put_u32(item, 20, checked_u32(name_len)?);
    put_u16(item, 24, label_count);
    put_u16(item, 26, 0);
    Ok(())
}

fn label_layout(
    fixed_end: usize,
    labels: &[(&[u8], &[u8])],
) -> Result<(usize, usize, usize), NipcError> {
    if labels.is_empty() {
        return Ok((fixed_end, 0, fixed_end));
    }
    let table_start = align8(fixed_end);
    let table_bytes = labels
        .len()
        .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
        .ok_or(NipcError::Overflow)?;
    let mut item_size = table_start
        .checked_add(table_bytes)
        .ok_or(NipcError::Overflow)?;
    for (key, value) in labels {
        if source_string_invalid(key, true) || source_string_invalid(value, false) {
            return Err(NipcError::BadLayout);
        }
        let key_size = key.len().checked_add(1).ok_or(NipcError::Overflow)?;
        let value_size = value.len().checked_add(1).ok_or(NipcError::Overflow)?;
        item_size = item_size
            .checked_add(key_size)
            .and_then(|v| v.checked_add(value_size))
            .ok_or(NipcError::Overflow)?;
    }
    Ok((table_start, table_bytes, item_size))
}

fn finish_lookup_response(
    buf: &mut [u8],
    hdr_size: usize,
    item_count: u32,
    data_offset: usize,
    generation: u64,
) -> Result<usize, NipcError> {
    put_u16(buf, 0, 1);
    put_u16(buf, 2, 0);
    put_u32(buf, 4, item_count);
    put_u64(buf, 8, generation);
    if item_count == 0 {
        return Ok(hdr_size);
    }
    let final_packed_start = (item_count as usize)
        .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
        .and_then(|v| hdr_size.checked_add(v))
        .ok_or(NipcError::Overflow)?;
    let first_item_abs = u32_at(buf, hdr_size) as usize;
    let packed_data_len = data_offset
        .checked_sub(first_item_abs)
        .ok_or(NipcError::Overflow)?;
    if final_packed_start < first_item_abs {
        let copy_end = first_item_abs
            .checked_add(packed_data_len)
            .ok_or(NipcError::Overflow)?;
        if copy_end > buf.len() {
            return Err(NipcError::Overflow);
        }
        let dest_end = final_packed_start
            .checked_add(packed_data_len)
            .ok_or(NipcError::Overflow)?;
        if dest_end > buf.len() {
            return Err(NipcError::Overflow);
        }
        buf.copy_within(first_item_abs..copy_end, final_packed_start);
    }
    for i in 0..item_count as usize {
        let entry_offset = i
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::Overflow)?;
        let entry = hdr_size
            .checked_add(entry_offset)
            .ok_or(NipcError::Overflow)?;
        let abs = u32_at(buf, entry) as usize;
        let rel = abs.checked_sub(first_item_abs).ok_or(NipcError::Overflow)?;
        put_u32(buf, entry, checked_u32(rel)?);
    }
    final_packed_start
        .checked_add(packed_data_len)
        .ok_or(NipcError::Overflow)
}

impl<'a> AppsLookupResponseView<'a> {
    pub fn decode(buf: &'a [u8]) -> Result<Self, NipcError> {
        if buf.len() < APPS_LOOKUP_RESP_HDR_SIZE {
            return Err(NipcError::Truncated);
        }
        if u16_at(buf, 0) != 1 || u16_at(buf, 2) != 0 {
            return Err(NipcError::BadLayout);
        }
        let item_count = u32_at(buf, 4);
        let dir_size = (item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::BadItemCount)?;
        let dir_end = APPS_LOOKUP_RESP_HDR_SIZE
            .checked_add(dir_size)
            .ok_or(NipcError::BadItemCount)?;
        if dir_end > buf.len() {
            return Err(NipcError::Truncated);
        }
        validate_lookup_dir(
            buf,
            APPS_LOOKUP_RESP_HDR_SIZE,
            item_count,
            buf.len() - dir_end,
            APPS_LOOKUP_ITEM_HDR_SIZE,
            None,
        )?;
        for i in 0..item_count as usize {
            let base = APPS_LOOKUP_RESP_HDR_SIZE + i * LOOKUP_DIR_ENTRY_SIZE;
            let off = u32_at(buf, base) as usize;
            let len = u32_at(buf, base + 4) as usize;
            decode_apps_item(checked_subslice(buf, dir_end, off, len)?)?;
        }
        Ok(Self {
            layout_version: 1,
            flags: 0,
            item_count,
            generation: u64_at(buf, 8),
            payload: buf,
        })
    }

    pub fn item(&self, index: u32) -> Result<AppsLookupItemView<'a>, NipcError> {
        if index >= self.item_count {
            return Err(NipcError::OutOfBounds);
        }
        let packed_start = lookup_data_offset(APPS_LOOKUP_RESP_HDR_SIZE, self.item_count)?;
        let base = lookup_dir_entry_offset(APPS_LOOKUP_RESP_HDR_SIZE, index)?;
        let off = u32_at(self.payload, base) as usize;
        let len = u32_at(self.payload, base + 4) as usize;
        decode_apps_item(checked_subslice(self.payload, packed_start, off, len)?)
    }
}

fn decode_apps_item(item: &[u8]) -> Result<AppsLookupItemView<'_>, NipcError> {
    if item.len() < APPS_LOOKUP_ITEM_HDR_SIZE {
        return Err(NipcError::Truncated);
    }
    let status = u16_at(item, 2);
    let orchestrator = u16_at(item, 4);
    let cgroup_status = u16_at(item, 6);
    let pid = u32_at(item, 8);
    let ppid = u32_at(item, 12);
    let uid = u32_at(item, 16);
    let starttime = u64_at(item, 24);
    let comm_off = u32_at(item, 32) as usize;
    let comm_len = u32_at(item, 36) as usize;
    let path_off = u32_at(item, 40) as usize;
    let path_len = u32_at(item, 44) as usize;
    let name_off = u32_at(item, 48) as usize;
    let name_len = u32_at(item, 52) as usize;
    let label_count = u16_at(item, 56);

    if u16_at(item, 0) != 1 || u32_at(item, 20) != 0 || u16_at(item, 58) != 0 {
        return Err(NipcError::BadLayout);
    }
    if status != PID_LOOKUP_KNOWN && status != PID_LOOKUP_UNKNOWN {
        return Err(NipcError::BadLayout);
    }
    if cgroup_status != APPS_CGROUP_KNOWN
        && cgroup_status != APPS_CGROUP_UNKNOWN_RETRY_LATER
        && cgroup_status != APPS_CGROUP_UNKNOWN_PERMANENT
        && cgroup_status != APPS_CGROUP_HOST_ROOT
    {
        return Err(NipcError::BadLayout);
    }
    if comm_len > 15 {
        return Err(NipcError::BadLayout);
    }
    if status == PID_LOOKUP_UNKNOWN {
        if orchestrator != 0
            || cgroup_status != 0
            || ppid != 0
            || uid != NIPC_UID_UNSET
            || starttime != 0
            || comm_len != 0
            || path_len != 0
            || name_len != 0
            || label_count != 0
        {
            return Err(NipcError::BadLayout);
        }
    } else if comm_len == 0 {
        return Err(NipcError::BadLayout);
    } else {
        match cgroup_status {
            APPS_CGROUP_KNOWN => {
                if path_len == 0 {
                    return Err(NipcError::BadLayout);
                }
            }
            APPS_CGROUP_UNKNOWN_RETRY_LATER | APPS_CGROUP_UNKNOWN_PERMANENT => {
                if path_len == 0 || orchestrator != 0 || name_len != 0 || label_count != 0 {
                    return Err(NipcError::BadLayout);
                }
            }
            APPS_CGROUP_HOST_ROOT => {
                if orchestrator != 0 || path_len != 0 || name_len != 0 || label_count != 0 {
                    return Err(NipcError::BadLayout);
                }
            }
            _ => return Err(NipcError::BadLayout),
        }
    }

    let (comm, comm_end) = lookup_string(item, APPS_LOOKUP_ITEM_HDR_SIZE, comm_off, comm_len)?;
    let (cgroup_path, path_end) =
        lookup_string(item, APPS_LOOKUP_ITEM_HDR_SIZE, path_off, path_len)?;
    let (cgroup_name, name_end) =
        lookup_string(item, APPS_LOOKUP_ITEM_HDR_SIZE, name_off, name_len)?;
    if overlap(comm_off, comm_end, path_off, path_end)
        || overlap(comm_off, comm_end, name_off, name_end)
        || overlap(path_off, path_end, name_off, name_end)
    {
        return Err(NipcError::BadLayout);
    }
    let label_table_offset = validate_labels(
        item,
        APPS_LOOKUP_ITEM_HDR_SIZE,
        label_count,
        comm_end.max(path_end).max(name_end),
    )?;
    Ok(AppsLookupItemView {
        status,
        orchestrator,
        cgroup_status,
        pid,
        ppid,
        uid,
        starttime,
        comm,
        cgroup_path,
        cgroup_name,
        label_count,
        item,
        label_table_offset,
    })
}

impl<'a> AppsLookupItemView<'a> {
    pub fn label(&self, index: u32) -> Result<LookupLabelView<'a>, NipcError> {
        label_at(
            self.item,
            APPS_LOOKUP_ITEM_HDR_SIZE,
            self.label_count,
            self.label_table_offset,
            index,
        )
    }
}

pub struct AppsLookupBuilder<'a> {
    buf: &'a mut [u8],
    generation: u64,
    item_count: u32,
    max_items: u32,
    data_offset: usize,
    error: Option<NipcError>,
}

impl<'a> AppsLookupBuilder<'a> {
    pub fn new(buf: &'a mut [u8], max_items: u32, generation: u64) -> Self {
        let data_offset = (max_items as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .and_then(|v| APPS_LOOKUP_RESP_HDR_SIZE.checked_add(v))
            .expect("AppsLookupBuilder buffer too small");
        assert!(
            buf.len() >= data_offset,
            "AppsLookupBuilder buffer too small"
        );
        Self {
            buf,
            generation,
            item_count: 0,
            max_items,
            data_offset,
            error: None,
        }
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        status: u16,
        cgroup_status: u16,
        orchestrator: u16,
        pid: u32,
        ppid: u32,
        uid: u32,
        starttime: u64,
        comm: &[u8],
        cgroup_path: &[u8],
        cgroup_name: &[u8],
        labels: &[(&[u8], &[u8])],
    ) -> Result<(), NipcError> {
        if self.item_count >= self.max_items {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        }
        if status != PID_LOOKUP_KNOWN && status != PID_LOOKUP_UNKNOWN {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        if cgroup_status != APPS_CGROUP_KNOWN
            && cgroup_status != APPS_CGROUP_UNKNOWN_RETRY_LATER
            && cgroup_status != APPS_CGROUP_UNKNOWN_PERMANENT
            && cgroup_status != APPS_CGROUP_HOST_ROOT
        {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        if comm.len() > 15
            || source_string_invalid(comm, status == PID_LOOKUP_KNOWN)
            || source_string_invalid(cgroup_path, false)
            || source_string_invalid(cgroup_name, false)
        {
            self.error = Some(NipcError::BadLayout);
            return Err(NipcError::BadLayout);
        }
        if status == PID_LOOKUP_UNKNOWN {
            if orchestrator != 0
                || cgroup_status != 0
                || ppid != 0
                || uid != NIPC_UID_UNSET
                || starttime != 0
                || !comm.is_empty()
                || !cgroup_path.is_empty()
                || !cgroup_name.is_empty()
                || !labels.is_empty()
            {
                self.error = Some(NipcError::BadLayout);
                return Err(NipcError::BadLayout);
            }
        } else {
            match cgroup_status {
                APPS_CGROUP_KNOWN => {
                    if cgroup_path.is_empty() {
                        self.error = Some(NipcError::BadLayout);
                        return Err(NipcError::BadLayout);
                    }
                }
                APPS_CGROUP_UNKNOWN_RETRY_LATER | APPS_CGROUP_UNKNOWN_PERMANENT => {
                    if cgroup_path.is_empty()
                        || orchestrator != 0
                        || !cgroup_name.is_empty()
                        || !labels.is_empty()
                    {
                        self.error = Some(NipcError::BadLayout);
                        return Err(NipcError::BadLayout);
                    }
                }
                APPS_CGROUP_HOST_ROOT => {
                    if orchestrator != 0
                        || !cgroup_path.is_empty()
                        || !cgroup_name.is_empty()
                        || !labels.is_empty()
                    {
                        self.error = Some(NipcError::BadLayout);
                        return Err(NipcError::BadLayout);
                    }
                }
                _ => unreachable!(),
            }
        }
        let label_count = match checked_u16(labels.len()) {
            Ok(v) => v,
            Err(err) => {
                self.error = Some(err);
                return Err(err);
            }
        };

        let item_start = align8(self.data_offset);
        let comm_offset = APPS_LOOKUP_ITEM_HDR_SIZE;
        let Some(path_offset) = comm_offset
            .checked_add(comm.len())
            .and_then(|v| v.checked_add(1))
        else {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        };
        let Some(name_offset) = path_offset
            .checked_add(cgroup_path.len())
            .and_then(|v| v.checked_add(1))
        else {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        };
        let Some(fixed_end) = name_offset
            .checked_add(cgroup_name.len())
            .and_then(|v| v.checked_add(1))
        else {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        };
        let (table_start, table_bytes, mut item_size) = label_layout(fixed_end, labels)?;
        let item_end = item_start
            .checked_add(item_size)
            .ok_or(NipcError::Overflow)?;
        if item_end > self.buf.len() {
            self.error = Some(NipcError::Overflow);
            return Err(NipcError::Overflow);
        }
        if item_start > self.data_offset {
            self.buf[self.data_offset..item_start].fill(0);
        }
        let item = &mut self.buf[item_start..item_end];
        if let Err(err) = write_apps_item_header(
            item,
            status,
            orchestrator,
            cgroup_status,
            pid,
            ppid,
            uid,
            starttime,
            comm_offset,
            comm.len(),
            path_offset,
            cgroup_path.len(),
            name_offset,
            cgroup_name.len(),
            label_count,
        ) {
            self.error = Some(err);
            return Err(err);
        }
        item[comm_offset..comm_offset + comm.len()].copy_from_slice(comm);
        item[comm_offset + comm.len()] = 0;
        item[path_offset..path_offset + cgroup_path.len()].copy_from_slice(cgroup_path);
        item[path_offset + cgroup_path.len()] = 0;
        item[name_offset..name_offset + cgroup_name.len()].copy_from_slice(cgroup_name);
        item[name_offset + cgroup_name.len()] = 0;
        if !labels.is_empty() {
            item[fixed_end..table_start].fill(0);
            let mut next = table_start
                .checked_add(table_bytes)
                .ok_or(NipcError::Overflow)?;
            for (i, (key, value)) in labels.iter().enumerate() {
                let entry_offset = i
                    .checked_mul(LOOKUP_LABEL_ENTRY_SIZE)
                    .ok_or(NipcError::Overflow)?;
                let entry = table_start
                    .checked_add(entry_offset)
                    .ok_or(NipcError::Overflow)?;
                let value_offset = next
                    .checked_add(key.len())
                    .and_then(|v| v.checked_add(1))
                    .ok_or(NipcError::Overflow)?;
                put_u32(item, entry, checked_u32(next)?);
                put_u32(item, entry + 4, checked_u32(key.len())?);
                put_u32(item, entry + 8, checked_u32(value_offset)?);
                put_u32(item, entry + 12, checked_u32(value.len())?);
                item[next..next + key.len()].copy_from_slice(key);
                item[next + key.len()] = 0;
                next = value_offset;
                item[next..next + value.len()].copy_from_slice(value);
                item[next + value.len()] = 0;
                next = next
                    .checked_add(value.len())
                    .and_then(|v| v.checked_add(1))
                    .ok_or(NipcError::Overflow)?;
            }
            item_size = next;
        }
        let dir_offset = (self.item_count as usize)
            .checked_mul(LOOKUP_DIR_ENTRY_SIZE)
            .ok_or(NipcError::Overflow)?;
        let dir = APPS_LOOKUP_RESP_HDR_SIZE
            .checked_add(dir_offset)
            .ok_or(NipcError::Overflow)?;
        put_u32(self.buf, dir, checked_u32(item_start)?);
        put_u32(self.buf, dir + 4, checked_u32(item_size)?);
        self.data_offset = item_end;
        self.item_count += 1;
        Ok(())
    }

    pub fn finish(self) -> Result<usize, NipcError> {
        finish_lookup_response(
            self.buf,
            APPS_LOOKUP_RESP_HDR_SIZE,
            self.item_count,
            self.data_offset,
            self.generation,
        )
    }

    pub fn error(&self) -> Option<NipcError> {
        self.error
    }

    pub fn item_count(&self) -> u32 {
        self.item_count
    }
}

#[allow(clippy::too_many_arguments)]
fn write_apps_item_header(
    item: &mut [u8],
    status: u16,
    orchestrator: u16,
    cgroup_status: u16,
    pid: u32,
    ppid: u32,
    uid: u32,
    starttime: u64,
    comm_off: usize,
    comm_len: usize,
    path_off: usize,
    path_len: usize,
    name_off: usize,
    name_len: usize,
    label_count: u16,
) -> Result<(), NipcError> {
    put_u16(item, 0, 1);
    put_u16(item, 2, status);
    put_u16(item, 4, orchestrator);
    put_u16(item, 6, cgroup_status);
    put_u32(item, 8, pid);
    put_u32(item, 12, ppid);
    put_u32(item, 16, uid);
    put_u32(item, 20, 0);
    put_u64(item, 24, starttime);
    put_u32(item, 32, checked_u32(comm_off)?);
    put_u32(item, 36, checked_u32(comm_len)?);
    put_u32(item, 40, checked_u32(path_off)?);
    put_u32(item, 44, checked_u32(path_len)?);
    put_u32(item, 48, checked_u32(name_off)?);
    put_u32(item, 52, checked_u32(name_len)?);
    put_u16(item, 56, label_count);
    put_u16(item, 58, 0);
    Ok(())
}

pub fn dispatch_cgroups_lookup<F>(
    req: &[u8],
    resp: &mut [u8],
    handler: F,
) -> Result<usize, NipcError>
where
    F: FnOnce(&CgroupsLookupRequestView, &mut CgroupsLookupBuilder) -> bool,
{
    let request = CgroupsLookupRequestView::decode(req)?;
    let min_required = lookup_data_offset(CGROUPS_LOOKUP_RESP_HDR_SIZE, request.item_count)
        .map_err(|_| NipcError::Overflow)?;
    if resp.len() < min_required {
        return Err(NipcError::Overflow);
    }
    let mut builder = CgroupsLookupBuilder::new(resp, request.item_count, 0);
    if !handler(&request, &mut builder) {
        return Err(builder.error().unwrap_or(NipcError::BadLayout));
    }
    if let Some(err) = builder.error() {
        return Err(err);
    }
    if builder.item_count != request.item_count {
        return Err(NipcError::BadItemCount);
    }
    builder.finish()
}

pub fn dispatch_apps_lookup<F>(req: &[u8], resp: &mut [u8], handler: F) -> Result<usize, NipcError>
where
    F: FnOnce(&AppsLookupRequestView, &mut AppsLookupBuilder) -> bool,
{
    let request = AppsLookupRequestView::decode(req)?;
    let min_required = lookup_data_offset(APPS_LOOKUP_RESP_HDR_SIZE, request.item_count)
        .map_err(|_| NipcError::Overflow)?;
    if resp.len() < min_required {
        return Err(NipcError::Overflow);
    }
    let mut builder = AppsLookupBuilder::new(resp, request.item_count, 0);
    if !handler(&request, &mut builder) {
        return Err(builder.error().unwrap_or(NipcError::BadLayout));
    }
    if let Some(err) = builder.error() {
        return Err(err);
    }
    if builder.item_count != request.item_count {
        return Err(NipcError::BadItemCount);
    }
    builder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ORCHESTRATOR_DOCKER, ORCHESTRATOR_K8S};

    #[test]
    fn cgroups_lookup_request_roundtrip() {
        let mut buf = [0u8; 128];
        let n = encode_cgroups_lookup_request(&[b"/a/b", b"/c"], &mut buf).unwrap();
        let view = CgroupsLookupRequestView::decode(&buf[..n]).unwrap();
        assert_eq!(view.item_count, 2);
        assert_eq!(view.item(0).unwrap().as_bytes(), b"/a/b");
        assert_eq!(view.item(1).unwrap().as_bytes(), b"/c");
    }

    #[test]
    fn cgroups_lookup_response_labels_roundtrip() {
        let mut buf = [0u8; 512];
        let mut b = CgroupsLookupBuilder::new(&mut buf, 1, 99);
        b.add(
            CGROUP_LOOKUP_KNOWN,
            ORCHESTRATOR_K8S,
            b"/kubepod",
            b"pod-a",
            &[(b"namespace".as_slice(), b"default".as_slice())],
        )
        .unwrap();
        let n = b.finish().unwrap();
        let view = CgroupsLookupResponseView::decode(&buf[..n]).unwrap();
        assert_eq!(view.generation, 99);
        let item = view.item(0).unwrap();
        assert_eq!(item.path.as_bytes(), b"/kubepod");
        assert_eq!(item.name.as_bytes(), b"pod-a");
        let label = item.label(0).unwrap();
        assert_eq!(label.key.as_bytes(), b"namespace");
        assert_eq!(label.value.as_bytes(), b"default");
    }

    #[test]
    fn apps_lookup_response_variants_roundtrip() {
        let mut buf = [0u8; 1024];
        let mut b = AppsLookupBuilder::new(&mut buf, 3, 7);
        b.add(
            PID_LOOKUP_KNOWN,
            APPS_CGROUP_KNOWN,
            ORCHESTRATOR_DOCKER,
            123,
            1,
            1000,
            42,
            b"nginx",
            b"/docker/abc",
            b"container-a",
            &[(b"image".as_slice(), b"nginx:latest".as_slice())],
        )
        .unwrap();
        b.add(
            PID_LOOKUP_KNOWN,
            APPS_CGROUP_HOST_ROOT,
            0,
            124,
            1,
            0,
            43,
            b"sshd",
            b"",
            b"",
            &[],
        )
        .unwrap();
        b.add(
            PID_LOOKUP_UNKNOWN,
            APPS_CGROUP_KNOWN,
            0,
            0,
            0,
            NIPC_UID_UNSET,
            0,
            b"",
            b"",
            b"",
            &[],
        )
        .unwrap();
        let n = b.finish().unwrap();
        let view = AppsLookupResponseView::decode(&buf[..n]).unwrap();
        assert_eq!(view.item_count, 3);
        assert_eq!(view.item(0).unwrap().comm.as_bytes(), b"nginx");
        assert_eq!(view.item(1).unwrap().cgroup_status, APPS_CGROUP_HOST_ROOT);
        assert_eq!(view.item(2).unwrap().status, PID_LOOKUP_UNKNOWN);
    }

    #[test]
    fn apps_lookup_comm_boundary() {
        let mut buf = [0u8; 256];
        let mut b = AppsLookupBuilder::new(&mut buf, 1, 0);
        assert!(b
            .add(
                PID_LOOKUP_KNOWN,
                APPS_CGROUP_HOST_ROOT,
                0,
                1,
                0,
                0,
                1,
                b"123456789012345",
                b"",
                b"",
                &[],
            )
            .is_ok());
        let mut b = AppsLookupBuilder::new(&mut buf, 1, 0);
        assert_eq!(
            b.add(
                PID_LOOKUP_KNOWN,
                APPS_CGROUP_HOST_ROOT,
                0,
                1,
                0,
                0,
                1,
                b"1234567890123456",
                b"",
                b"",
                &[],
            )
            .unwrap_err(),
            NipcError::BadLayout
        );
    }

    fn cgroups_lookup_labeled_response() -> Vec<u8> {
        let mut buf = vec![0u8; 512];
        let mut b = CgroupsLookupBuilder::new(&mut buf, 1, 1);
        b.add(
            CGROUP_LOOKUP_KNOWN,
            ORCHESTRATOR_K8S,
            b"/x",
            b"n",
            &[(b"k".as_slice(), b"v".as_slice())],
        )
        .unwrap();
        let n = b.finish().unwrap();
        buf.truncate(n);
        buf
    }

    fn apps_lookup_host_root_response() -> Vec<u8> {
        let mut buf = vec![0u8; 256];
        let mut b = AppsLookupBuilder::new(&mut buf, 1, 1);
        b.add(
            PID_LOOKUP_KNOWN,
            APPS_CGROUP_HOST_ROOT,
            0,
            123,
            1,
            1000,
            42,
            b"a",
            b"",
            b"",
            &[],
        )
        .unwrap();
        let n = b.finish().unwrap();
        buf.truncate(n);
        buf
    }

    fn response_item_bounds(
        buf: &[u8],
        hdr_size: usize,
        item_count: usize,
        index: usize,
    ) -> (usize, usize) {
        let dir = hdr_size + index * LOOKUP_DIR_ENTRY_SIZE;
        let off = u32_at(buf, dir) as usize;
        let len = u32_at(buf, dir + 4) as usize;
        (hdr_size + item_count * LOOKUP_DIR_ENTRY_SIZE + off, len)
    }

    #[test]
    fn lookup_empty_requests_responses() {
        let mut buf = [0u8; 64];
        let n = encode_cgroups_lookup_request(&[], &mut buf).unwrap();
        let view = CgroupsLookupRequestView::decode(&buf[..n]).unwrap();
        assert_eq!(view.item_count, 0);

        let n = encode_apps_lookup_request(&[], &mut buf).unwrap();
        let view = AppsLookupRequestView::decode(&buf[..n]).unwrap();
        assert_eq!(view.item_count, 0);

        let mut cbuf = [0u8; 64];
        let b = CgroupsLookupBuilder::new(&mut cbuf, 0, 9);
        let n = b.finish().unwrap();
        let view = CgroupsLookupResponseView::decode(&cbuf[..n]).unwrap();
        assert_eq!(view.item_count, 0);
        assert_eq!(view.generation, 9);

        let mut abuf = [0u8; 64];
        let b = AppsLookupBuilder::new(&mut abuf, 0, 10);
        let n = b.finish().unwrap();
        let view = AppsLookupResponseView::decode(&abuf[..n]).unwrap();
        assert_eq!(view.item_count, 0);
        assert_eq!(view.generation, 10);
    }

    #[test]
    fn lookup_dispatch_rejects_short_response_buffer() {
        let mut req = [0u8; 64];
        let n = encode_cgroups_lookup_request(&[b"/x"], &mut req).unwrap();
        let mut resp = vec![0u8; CGROUPS_LOOKUP_RESP_HDR_SIZE + LOOKUP_DIR_ENTRY_SIZE - 1];
        assert_eq!(
            dispatch_cgroups_lookup(&req[..n], &mut resp, |_, _| {
                panic!("handler must not run with undersized response buffer")
            })
            .unwrap_err(),
            NipcError::Overflow
        );

        let n = encode_apps_lookup_request(&[1234], &mut req).unwrap();
        let mut resp = vec![0u8; APPS_LOOKUP_RESP_HDR_SIZE + LOOKUP_DIR_ENTRY_SIZE - 1];
        assert_eq!(
            dispatch_apps_lookup(&req[..n], &mut resp, |_, _| {
                panic!("handler must not run with undersized response buffer")
            })
            .unwrap_err(),
            NipcError::Overflow
        );
    }

    #[test]
    fn lookup_requests_reject_bad_layouts() {
        let mut buf = [0u8; 128];
        let n = encode_cgroups_lookup_request(&[b"/x"], &mut buf).unwrap();
        assert_eq!(
            CgroupsLookupRequestView::decode(&buf[..CGROUPS_LOOKUP_REQ_HDR_SIZE - 1]).unwrap_err(),
            NipcError::Truncated
        );

        let mut bad = buf[..n].to_vec();
        put_u16(&mut bad, 0, 99);
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, 8, 1);
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, CGROUPS_LOOKUP_REQ_HDR_SIZE, 1);
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadAlignment
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, CGROUPS_LOOKUP_REQ_HDR_SIZE + 4, 4096);
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::OutOfBounds
        );

        bad.copy_from_slice(&buf[..n]);
        let last = bad.len() - 1;
        bad[last] = b'x';
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::MissingNul
        );

        bad.copy_from_slice(&buf[..n]);
        bad[CGROUPS_LOOKUP_REQ_HDR_SIZE + LOOKUP_DIR_ENTRY_SIZE] = 0;
        assert_eq!(
            CgroupsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        let n = encode_apps_lookup_request(&[1234], &mut buf).unwrap();
        bad = buf[..n].to_vec();
        put_u32(&mut bad, 8, 1);
        assert_eq!(
            AppsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, APPS_LOOKUP_REQ_HDR_SIZE, 1);
        assert_eq!(
            AppsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadAlignment
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, APPS_LOOKUP_REQ_HDR_SIZE + 4, 7);
        assert_eq!(
            AppsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(&mut bad, APPS_LOOKUP_REQ_HDR_SIZE, 8);
        assert_eq!(
            AppsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::OutOfBounds
        );

        bad.copy_from_slice(&buf[..n]);
        put_u32(
            &mut bad,
            APPS_LOOKUP_REQ_HDR_SIZE + LOOKUP_DIR_ENTRY_SIZE + 4,
            1,
        );
        assert_eq!(
            AppsLookupRequestView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );
    }

    #[test]
    fn cgroups_lookup_response_rejects_bad_layouts() {
        let buf = cgroups_lookup_labeled_response();
        assert_eq!(
            CgroupsLookupResponseView::decode(&buf[..CGROUPS_LOOKUP_RESP_HDR_SIZE - 1])
                .unwrap_err(),
            NipcError::Truncated
        );

        let mut bad = buf.clone();
        put_u16(&mut bad, 0, 99);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        put_u16(&mut bad, 2, 1);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        put_u32(&mut bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadAlignment
        );

        bad = buf.clone();
        put_u32(&mut bad, CGROUPS_LOOKUP_RESP_HDR_SIZE + 4, 4096);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::OutOfBounds
        );

        bad = buf.clone();
        put_u32(
            &mut bad,
            CGROUPS_LOOKUP_RESP_HDR_SIZE + 4,
            (CGROUPS_LOOKUP_ITEM_HDR_SIZE - 1) as u32,
        );
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        put_u16(&mut bad, item_start, 99);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        let path_off = u32_at(&bad, item_start + 8) as usize;
        let path_len = u32_at(&bad, item_start + 12) as usize;
        bad[item_start + path_off + path_len] = b'x';
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::MissingNul
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        put_u32(&mut bad, item_start + 8, 4);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::OutOfBounds
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        let path_off = u32_at(&bad, item_start + 8);
        let path_len = u32_at(&bad, item_start + 12);
        put_u32(&mut bad, item_start + 16, path_off);
        put_u32(&mut bad, item_start + 20, path_len);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        let path_off = u32_at(&bad, item_start + 8) as usize;
        let path_len = u32_at(&bad, item_start + 12) as usize;
        let name_off = u32_at(&bad, item_start + 16) as usize;
        let name_len = u32_at(&bad, item_start + 20) as usize;
        let fixed_end = (path_off + path_len + 1).max(name_off + name_len + 1);
        bad[item_start + fixed_end] = 1;
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, CGROUPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        let path_off = u32_at(&bad, item_start + 8) as usize;
        let path_len = u32_at(&bad, item_start + 12) as usize;
        let name_off = u32_at(&bad, item_start + 16) as usize;
        let name_len = u32_at(&bad, item_start + 20) as usize;
        let fixed_end = (path_off + path_len + 1).max(name_off + name_len + 1);
        let table_start = align8(fixed_end);
        put_u32(&mut bad, item_start + table_start + 4, 0);
        assert_eq!(
            CgroupsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        let mut two = vec![0u8; 512];
        let mut b = CgroupsLookupBuilder::new(&mut two, 2, 1);
        b.add(CGROUP_LOOKUP_UNKNOWN_PERMANENT, 0, b"/a", b"", &[])
            .unwrap();
        b.add(CGROUP_LOOKUP_UNKNOWN_PERMANENT, 0, b"/b", b"", &[])
            .unwrap();
        let n = b.finish().unwrap();
        two.truncate(n);
        let first_off = u32_at(&two, CGROUPS_LOOKUP_RESP_HDR_SIZE);
        put_u32(
            &mut two,
            CGROUPS_LOOKUP_RESP_HDR_SIZE + LOOKUP_DIR_ENTRY_SIZE,
            first_off,
        );
        assert_eq!(
            CgroupsLookupResponseView::decode(&two).unwrap_err(),
            NipcError::BadLayout
        );
    }

    #[test]
    fn apps_lookup_response_rejects_bad_layouts() {
        let buf = apps_lookup_host_root_response();

        let mut bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, APPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        put_u16(&mut bad, item_start + 2, 99);
        assert_eq!(
            AppsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, APPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        put_u16(&mut bad, item_start + 6, 99);
        assert_eq!(
            AppsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );

        bad = buf.clone();
        let (item_start, _) = response_item_bounds(&bad, APPS_LOOKUP_RESP_HDR_SIZE, 1, 0);
        put_u32(&mut bad, item_start + 36, 0);
        assert_eq!(
            AppsLookupResponseView::decode(&bad).unwrap_err(),
            NipcError::BadLayout
        );
    }
}
