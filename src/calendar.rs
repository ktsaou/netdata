// SPDX-License-Identifier: GPL-3.0-or-later
// UTC absolute-day arithmetic follows Go 1.27.1 time/time.go; see NOTICE.

const ABSOLUTE_YEARS: u64 = 292_277_022_400;
const UNIX_TO_ABSOLUTE: i64 = 9_223_372_028_741_760_000;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CalendarFunction {
    DaysInMonth,
    DayOfMonth,
    DayOfWeek,
    DayOfYear,
    Hour,
    Minute,
    Month,
    Year,
}
impl CalendarFunction {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "days_in_month" => Self::DaysInMonth,
            "day_of_month" => Self::DayOfMonth,
            "day_of_week" => Self::DayOfWeek,
            "day_of_year" => Self::DayOfYear,
            "hour" => Self::Hour,
            "minute" => Self::Minute,
            "month" => Self::Month,
            "year" => Self::Year,
            _ => return None,
        })
    }
    pub(crate) fn apply(self, seconds: i64) -> f64 {
        let absolute = seconds.wrapping_add(UNIX_TO_ABSOLUTE) as u64;
        let days = absolute / 86_400;
        let date = Date::from_days(days);
        match self {
            Self::DaysInMonth => {
                // The source Date(year, month, 32) round trip can wrap absolute seconds.
                let next = days_for_date(date.year, date.month, 32).wrapping_mul(86_400);
                f64::from(32 - Date::from_days(next / 86_400).day)
            }
            Self::DayOfMonth => f64::from(date.day),
            Self::DayOfWeek => ((days + 3) % 7) as f64,
            Self::DayOfYear => f64::from(date.year_day),
            Self::Hour => ((absolute % 86_400) / 3600) as f64,
            Self::Minute => ((absolute % 3600) / 60) as f64,
            Self::Month => f64::from(date.month),
            Self::Year => date.year as f64,
        }
    }
}

pub(crate) fn float_seconds(value: f64) -> i64 {
    // Go amd64's out-of-range conversion returns integer-indefinite, including NaNs.
    if (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&value) {
        value as i64
    } else {
        i64::MIN
    }
}

pub(crate) fn write_rfc3339(seconds: i64, out: &mut impl std::fmt::Write) -> std::fmt::Result {
    let absolute = seconds.wrapping_add(UNIX_TO_ABSOLUTE) as u64;
    let date = Date::from_days(absolute / 86_400);
    if date.year < 0 {
        out.write_str("-")?;
    }
    // Go pads the year magnitude and accepts years beyond the strict RFC3339 range.
    write!(
        out,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        date.year.unsigned_abs(),
        date.month,
        date.day,
        (absolute % 86_400) / 3600,
        (absolute % 3600) / 60,
        absolute % 60,
    )
}

struct Date {
    year: i64,
    month: u32,
    day: u32,
    year_day: u32,
}
impl Date {
    fn from_days(days: u64) -> Self {
        let d = 4 * days + 3;
        let century = d / 146_097;
        let century_day = (d % 146_097) as u32 | 3;
        let product = 2_939_745_u64 * u64::from(century_day);
        let century_year = (product >> 32) as u32;
        let absolute_day = (product as u32) / 2_939_745 / 4;
        let split = 2141 * absolute_day + 197_913;
        let jan_feb = u32::from(absolute_day >= 306);
        let leap = u32::from(
            century_year.is_multiple_of(4) && (century_year != 0 || century.is_multiple_of(4)),
        );
        Self {
            year: century.wrapping_mul(100).wrapping_sub(ABSOLUTE_YEARS) as i64
                + i64::from(century_year)
                + i64::from(jan_feb),
            month: (split >> 16) - jan_feb * 12,
            day: 1 + (split & 0xffff) / 2141,
            year_day: absolute_day + 60 + (leap & !jan_feb) - 365 * jan_feb,
        }
    }
}

fn days_for_date(year: i64, month: u32, day: i64) -> u64 {
    let jan_feb = u32::from(month < 3);
    let absolute_month = month + 12 * jan_feb;
    let year = (year as u64)
        .wrapping_sub(u64::from(jan_feb))
        .wrapping_add(ABSOLUTE_YEARS);
    let absolute_day = (979 * absolute_month - 2919) >> 5;
    let century = year / 100;
    let century_year = (year % 100) as u32;
    let century_day = 1461 * century_year / 4;
    let century_days = 146_097_u64.wrapping_mul(century) / 4;
    century_days.wrapping_add((i64::from(century_day + absolute_day) + day - 1) as u64)
}

#[cfg(any(test, feature = "fixtures"))]
pub fn fixture(value: f64) -> (i64, [f64; 8]) {
    use CalendarFunction::*;
    let seconds = float_seconds(value);
    (
        seconds,
        [
            Year,
            Month,
            DayOfMonth,
            DayOfYear,
            DayOfWeek,
            Hour,
            Minute,
            DaysInMonth,
        ]
        .map(|function| function.apply(seconds)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_extreme_seconds_keep_conversion_and_absolute_wrap() {
        let sample = f64::from_bits(0xc3df_ffff_ff87_1b4e);
        let seconds = float_seconds(sample);
        assert_eq!(CalendarFunction::Year.apply(seconds), 292_277_026_854.0);
        assert_eq!(CalendarFunction::Month.apply(seconds), 1.0);
        assert_eq!(CalendarFunction::DaysInMonth.apply(seconds), 7.0);
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(float_seconds(value), i64::MIN);
            assert_eq!(
                CalendarFunction::Year.apply(float_seconds(value)),
                292_277_026_596.0
            );
            assert_eq!(CalendarFunction::Month.apply(float_seconds(value)), 12.0);
            assert_eq!(
                CalendarFunction::DayOfMonth.apply(float_seconds(value)),
                4.0
            );
        }
        assert_eq!(CalendarFunction::DayOfMonth.apply(0), 1.0);
        assert_eq!(CalendarFunction::Month.apply(0), 1.0);
        assert_eq!(CalendarFunction::Year.apply(0), 1970.0);
    }
}
