// SPDX-License-Identifier: GPL-3.0-or-later
// UTC wall-clock operations follow pinned Go time.Time; see NOTICE.
use crate::{evaluator::EvaluationError, result::OutputHeader};

const SECOND: i64 = 1_000_000_000;
const UNIX_TO_INTERNAL: i64 = 62_135_596_800;

/// Normalized UTC timestamp; unlike Unix nanoseconds, this also represents large RFC3339 years.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    pub unix_seconds: i64,
    pub nanoseconds: u32,
}
impl Timestamp {
    pub fn from_unix_parts(seconds: i64, nanoseconds: i64) -> Self {
        Self {
            unix_seconds: seconds.wrapping_add(nanoseconds.div_euclid(SECOND)),
            nanoseconds: nanoseconds.rem_euclid(SECOND) as u32,
        }
    }
    pub fn unix_milliseconds(self) -> i64 {
        self.unix_seconds
            .wrapping_mul(1000)
            .wrapping_add(i64::from(self.nanoseconds) / 1_000_000)
    }
    pub fn evaluator_milliseconds(self) -> i64 {
        self.unix_seconds
            .wrapping_mul(SECOND)
            .wrapping_add(i64::from(self.nanoseconds))
            / 1_000_000
    }
    fn internal_seconds(self) -> i64 {
        self.unix_seconds.wrapping_add(UNIX_TO_INTERNAL)
    }
    pub fn before(self, other: Self) -> bool {
        (self.internal_seconds(), self.nanoseconds) < (other.internal_seconds(), other.nanoseconds)
    }
    fn add_duration(self, duration_ns: i64) -> Self {
        let nanos = i64::from(self.nanoseconds) + duration_ns % SECOND;
        let seconds = (duration_ns / SECOND) + nanos.div_euclid(SECOND);
        let internal = self.internal_seconds();
        let sum = internal.wrapping_add(seconds);
        let internal = if (sum > internal) == (seconds > 0) {
            sum
        } else if seconds > 0 {
            i64::MAX
        } else {
            -i64::MAX
        };
        Self {
            unix_seconds: internal.wrapping_sub(UNIX_TO_INTERNAL),
            nanoseconds: nanos.rem_euclid(SECOND) as u32,
        }
    }
    pub fn duration_since(self, other: Self) -> i64 {
        let duration = self
            .internal_seconds()
            .wrapping_sub(other.internal_seconds())
            .wrapping_mul(SECOND)
            .wrapping_add(i64::from(self.nanoseconds) - i64::from(other.nanoseconds));
        if other.add_duration(duration) == self {
            duration
        } else if self.before(other) {
            i64::MIN
        } else {
            i64::MAX
        }
    }
    fn validate(self) -> Result<(), EvaluationError> {
        if self.nanoseconds >= SECOND as u32 {
            Err(EvaluationError::Invalid(
                "timestamp nanoseconds are not normalized",
            ))
        } else {
            Ok(())
        }
    }
}

/// The caller resolves lookback against configuration before constructing this request.
#[derive(Clone, Copy, Debug)]
pub enum PreciseQueryRequest {
    Instant {
        time: Timestamp,
        lookback_ns: i64,
    },
    Range {
        start: Timestamp,
        end: Timestamp,
        interval_ns: i64,
        lookback_ns: i64,
    },
}

#[derive(Clone, Copy)]
pub(crate) enum Request {
    Milliseconds(crate::execution::QueryRequest),
    Precise(PreciseQueryRequest),
}
impl From<crate::execution::QueryRequest> for Request {
    fn from(request: crate::execution::QueryRequest) -> Self {
        Self::Milliseconds(request)
    }
}
impl From<PreciseQueryRequest> for Request {
    fn from(request: PreciseQueryRequest) -> Self {
        Self::Precise(request)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Timing {
    pub(crate) start_ms: i64,
    pub(crate) end_ms: i64,
    pub(crate) evaluator_start_ms: i64,
    pub(crate) evaluator_end_ms: i64,
    pub(crate) interval_ns: i64,
    pub(crate) range_ns: i64,
    pub(crate) same_time: bool,
    pub(crate) lookback_ns: Option<i64>,
}
impl Timing {
    pub(crate) fn milliseconds(header: OutputHeader) -> Self {
        let range = (i128::from(header.end_ms) - i128::from(header.start_ms)) * 1_000_000;
        Self {
            start_ms: header.start_ms,
            end_ms: header.end_ms,
            evaluator_start_ms: header.start_ms.wrapping_mul(1_000_000) / 1_000_000,
            evaluator_end_ms: header.end_ms.wrapping_mul(1_000_000) / 1_000_000,
            interval_ns: header.step_ms.wrapping_mul(1_000_000),
            range_ns: range.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64,
            same_time: header.start_ms == header.end_ms,
            lookback_ns: None,
        }
    }
    pub(crate) fn precise(request: PreciseQueryRequest) -> Result<Self, EvaluationError> {
        let (start, end, interval_ns, lookback_ns) = match request {
            PreciseQueryRequest::Instant { time, lookback_ns } => (time, time, 0, lookback_ns),
            PreciseQueryRequest::Range {
                start,
                end,
                interval_ns,
                lookback_ns,
            } => {
                if interval_ns <= 0 || end.before(start) {
                    return Err(EvaluationError::Invalid("invalid range query times"));
                }
                (start, end, interval_ns, lookback_ns)
            }
        };
        start.validate()?;
        end.validate()?;
        Ok(Self {
            start_ms: start.unix_milliseconds(),
            end_ms: end.unix_milliseconds(),
            evaluator_start_ms: start.evaluator_milliseconds(),
            evaluator_end_ms: end.evaluator_milliseconds(),
            interval_ns,
            range_ns: end.duration_since(start),
            same_time: start == end,
            lookback_ns: Some(lookback_ns),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_clock_projections_and_saturation() {
        let minus_ns = Timestamp::from_unix_parts(0, -1);
        assert_eq!(minus_ns.unix_milliseconds(), -1);
        assert_eq!(minus_ns.evaluator_milliseconds(), 0);
        assert_eq!(
            Timestamp::from_unix_parts(0, 2).duration_since(Timestamp::from_unix_parts(0, 1)),
            1
        );
        let year9999 = Timestamp::from_unix_parts(253_402_300_799, 999_999_999);
        assert_eq!(year9999.unix_milliseconds(), 253_402_300_799_999);
        assert_eq!(
            year9999.duration_since(Timestamp::from_unix_parts(0, 0)),
            i64::MAX
        );
        assert_eq!(
            Timestamp::from_unix_parts(0, 0).duration_since(year9999),
            i64::MIN
        );
    }
}
