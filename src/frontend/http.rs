// SPDX-License-Identifier: GPL-3.0-or-later
// Prometheus web/api/v1 and Go RFC3339 fallback semantics; see NOTICE.
use super::{float, number, quote, work::Work};
use crate::{
    accounting::{AccountedVec, AllocationContext, QueryAccounting},
    query_time::Timestamp,
    storage::{Control, Interrupted},
};

pub const MIN_TIME: Timestamp = Timestamp {
    unix_seconds: -9_223_309_901_257_974,
    nanoseconds: 0,
};
pub const MAX_TIME: Timestamp = Timestamp {
    unix_seconds: 9_223_309_901_257_974,
    nanoseconds: 999_999_999,
};

pub fn parse_time(
    input: &[u8],
    account: &QueryAccounting,
    control: &Control,
) -> Result<Result<Timestamp, AccountedVec<u8>>, Interrupted> {
    let context = AllocationContext::new(account, crate::accounting::MemoryKind::Workspace);
    let parsed = float::parse_checked(input, Some(&context), Some(account), Some(control))?;
    if parsed.error.is_none() {
        let seconds = parsed.value.trunc();
        let fraction = (parsed.value - seconds) * 1000.0;
        let nanos = (fraction.round() / 1000.0) * 1_000_000_000.0;
        return Ok(Ok(Timestamp::from_unix_parts(
            number::go_i64(seconds),
            number::go_i64(nanos),
        )));
    }
    let work = Work::new(Some(&context), Some(control));
    if let Some(value) = match input {
        b"-292273086-05-16T16:47:06Z" => Some(MIN_TIME),
        b"292277025-08-18T07:12:54.999999999Z" => Some(MAX_TIME),
        _ => rfc3339(input, work)?,
    } {
        return Ok(Ok(value));
    }
    Ok(Err(work.format(format_args!(
        "cannot parse {} to a valid timestamp",
        quote::quoted(input)
    ))?))
}

pub fn parse_duration(
    input: &[u8],
    account: &QueryAccounting,
    control: &Control,
) -> Result<Result<i64, AccountedVec<u8>>, Interrupted> {
    let context = AllocationContext::new(account, crate::accounting::MemoryKind::Workspace);
    let work = Work::new(Some(&context), Some(control));
    let parsed = float::parse_checked(input, Some(&context), Some(account), Some(control))?;
    if parsed.error.is_none() {
        let duration = parsed.value * 1_000_000_000.0;
        if duration > i64::MAX as f64 || duration < i64::MIN as f64 {
            return Ok(Err(work.format(format_args!(
                "cannot parse {} to a valid duration. It overflows int64",
                quote::quoted(input)
            ))?));
        }
        return Ok(Ok(number::go_i64(duration)));
    }
    if let Ok(value) = number::duration_in(input, work)? {
        return Ok(Ok(value));
    }
    Ok(Err(work.format(format_args!(
        "cannot parse {} to a valid duration",
        quote::quoted(input)
    ))?))
}

fn digits(input: &[u8], begin: usize, count: usize) -> Option<u32> {
    let mut value = 0;
    for &byte in input.get(begin..begin.checked_add(count)?)? {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + u32::from(byte - b'0');
    }
    Some(value)
}

fn rfc3339(input: &[u8], work: Work<'_>) -> Result<Option<Timestamp>, Interrupted> {
    let mut interrupted = None;
    let result = (|| {
        if input.get(4) != Some(&b'-')
            || input.get(7) != Some(&b'-')
            || input.get(10) != Some(&b'T')
        {
            return None;
        }
        let year = digits(input, 0, 4)?;
        let month = digits(input, 5, 2)?;
        let day = digits(input, 8, 2)?;
        if !(1..=12).contains(&month) {
            return None;
        }
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days_in_month = [
            31,
            28 + u32::from(leap),
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > days_in_month[(month - 1) as usize] {
            return None;
        }
        let hour_width = if input.get(12) == Some(&b':') { 1 } else { 2 };
        let colon = 11 + hour_width;
        if input.get(colon) != Some(&b':') || input.get(colon + 3) != Some(&b':') {
            return None;
        }
        let hour = digits(input, 11, hour_width)?;
        let minute = digits(input, colon + 1, 2)?;
        let second = digits(input, colon + 4, 2)?;
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        let mut index = colon + 6;
        let mut nanoseconds = 0;
        if matches!(input.get(index), Some(b'.' | b',')) {
            index += 1;
            let start = index;
            while input.get(index).is_some_and(u8::is_ascii_digit) {
                if (index - start).is_multiple_of(1024)
                    && let Err(error) = work.step(1024.min(input.len() - index))
                {
                    interrupted = Some(error);
                    return None;
                }
                if index - start < 9 {
                    nanoseconds = nanoseconds * 10 + u32::from(input[index] - b'0');
                }
                index += 1;
            }
            if index == start {
                return None;
            }
            for _ in (index - start).min(9)..9 {
                nanoseconds *= 10;
            }
        }
        let offset = match input.get(index)? {
            b'Z' if index + 1 == input.len() => 0,
            sign @ (b'+' | b'-')
                if index + 6 == input.len() && input.get(index + 3) == Some(&b':') =>
            {
                let hours = digits(input, index + 1, 2)?;
                let minutes = digits(input, index + 4, 2)?;
                // Go's fallback parser admits 24 hours and 60 minutes.
                if hours > 24 || minutes > 60 {
                    return None;
                }
                let value = i64::from(hours * 3600 + minutes * 60);
                if *sign == b'-' { -value } else { value }
            }
            _ => return None,
        };
        let previous_year = i64::from(year) - 1;
        let days_before_year = 365 * previous_year + previous_year.div_euclid(4)
            - previous_year.div_euclid(100)
            + previous_year.div_euclid(400);
        let days_before_month: u32 = days_in_month[..(month - 1) as usize].iter().sum();
        let days = days_before_year + i64::from(days_before_month + day - 1) - 719_162;
        Some(Timestamp {
            unix_seconds: days * 86_400 + i64::from(hour * 3600 + minute * 60 + second) - offset,
            nanoseconds,
        })
    })();
    if let Some(error) = interrupted {
        Err(error)
    } else {
        work.step(0)?;
        Ok(result)
    }
}
