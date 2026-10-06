// SPDX-License-Identifier: GPL-3.0-or-later
// Go 1.27.1 internal/strconv/atof.go and atoi.go; see NOTICE.
use super::{float_decimal, float_scale, work::Work};
use crate::{
    accounting::{AllocationContext, QueryAccounting},
    storage::{Control, Interrupted},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FloatError {
    Syntax,
    Range,
}

impl FloatError {
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Syntax => "invalid syntax",
            Self::Range => "value out of range",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ParsedFloat {
    pub(crate) value: f64,
    pub(crate) error: Option<FloatError>,
}

impl ParsedFloat {
    fn syntax() -> Self {
        Self {
            value: 0.0,
            error: Some(FloatError::Syntax),
        }
    }
}

pub(super) fn digit(byte: u8) -> u32 {
    match byte {
        b'0'..=b'9' => u32::from(byte - b'0'),
        b'a'..=b'f' => u32::from(byte - b'a' + 10),
        b'A'..=b'F' => u32::from(byte - b'A' + 10),
        _ => 16,
    }
}

fn exact_decimal(mantissa: u64, exponent: i32, negative: bool) -> Option<f64> {
    const POWERS: [f64; 23] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16,
        1e17, 1e18, 1e19, 1e20, 1e21, 1e22,
    ];
    if mantissa >> 52 != 0 {
        return None;
    }
    let mut value = mantissa as f64;
    if negative {
        value = -value;
    }
    match exponent {
        0 => Some(value),
        1..=37 => {
            let mut exponent = exponent;
            if exponent > 22 {
                value *= POWERS[(exponent - 22) as usize];
                exponent = 22;
            }
            if !(-1e15..=1e15).contains(&value) {
                return None;
            }
            Some(value * POWERS[exponent as usize])
        }
        -22..=-1 => Some(value / POWERS[(-exponent) as usize]),
        _ => None,
    }
}

fn decimal(input: &[u8], sign: usize, work: Work<'_>) -> Result<ParsedFloat, Interrupted> {
    let negative = input.first() == Some(&b'-');
    let mut position = sign;
    let mut mantissa = 0u64;
    let mut stored = 0;
    let mut digits = 0i128;
    let mut point = 0i128;
    let mut saw_digits = false;
    let mut saw_dot = false;
    let mut truncated = false;
    let mut underscores = false;
    while let Some(&byte) = input.get(position) {
        work.step(1)?;
        if byte == b'_' {
            underscores = true;
        } else if byte == b'.' {
            if saw_dot {
                break;
            }
            saw_dot = true;
            point = digits;
        } else if byte.is_ascii_digit() {
            saw_digits = true;
            if byte == b'0' && digits == 0 {
                point -= 1;
            } else {
                digits += 1;
                if stored < 19 {
                    mantissa = mantissa * 10 + u64::from(byte - b'0');
                    stored += 1;
                } else if byte != b'0' {
                    truncated = true;
                }
            }
        } else {
            break;
        }
        position += 1;
    }
    if !saw_digits {
        return Ok(ParsedFloat::syntax());
    }
    if !saw_dot {
        point = digits;
    }
    let mantissa_end = position;
    let mut exponent = 0i128;
    if matches!(input.get(position), Some(b'e' | b'E')) {
        position += 1;
        let negative_exponent = input.get(position) == Some(&b'-');
        if matches!(input.get(position), Some(b'+' | b'-')) {
            position += 1;
        }
        if !input.get(position).is_some_and(u8::is_ascii_digit) {
            return Ok(ParsedFloat::syntax());
        }
        while let Some(&byte) = input.get(position) {
            work.step(1)?;
            if byte == b'_' {
                underscores = true;
            } else if byte.is_ascii_digit() {
                // This freezes a prefix; it does not clamp the final exponent to 10000.
                if exponent < 10000 {
                    exponent = exponent * 10 + i128::from(byte - b'0');
                }
            } else {
                break;
            }
            position += 1;
        }
        if negative_exponent {
            exponent = -exponent;
        }
    }
    if position != input.len() {
        return Ok(ParsedFloat::syntax());
    }
    if underscores {
        for (index, &byte) in input.iter().enumerate() {
            work.step(1)?;
            if byte == b'_'
                && (index == 0
                    || !input[index - 1].is_ascii_digit()
                    || input.get(index + 1).is_none_or(|b| !b.is_ascii_digit()))
            {
                return Ok(ParsedFloat::syntax());
            }
        }
    }
    let sign_bits = u64::from(negative) << 63;
    let power = point + exponent - stored;
    if mantissa == 0 || power < -345 {
        return Ok(ParsedFloat {
            value: f64::from_bits(sign_bits),
            error: None,
        });
    }
    if power > 310 {
        return Ok(ParsedFloat {
            value: f64::from_bits(sign_bits | (0x7ff << 52)),
            error: Some(FloatError::Range),
        });
    }
    work.step(1)?;
    if !truncated && let Some(value) = exact_decimal(mantissa, power as i32, negative) {
        return Ok(ParsedFloat { value, error: None });
    }
    let parsed = float_scale::parse(mantissa, power as i32, sign_bits);
    if !truncated {
        return Ok(parsed);
    }
    work.step(1)?;
    if parsed.value == float_scale::parse(mantissa + 1, power as i32, sign_bits).value {
        return Ok(parsed);
    }
    float_decimal::parse(&input[sign..mantissa_end], exponent, sign_bits, work)
}

fn underscore_ok(input: &[u8], start: usize, work: Work<'_>) -> Result<bool, Interrupted> {
    let mut previous_digit = true; // The hexadecimal prefix counts as a digit.
    let mut pending = false;
    for &byte in &input[start..] {
        work.step(1)?;
        if digit(byte) < 16 {
            previous_digit = true;
            pending = false;
        } else if byte == b'_' {
            if !previous_digit {
                return Ok(false);
            }
            previous_digit = false;
            pending = true;
        } else {
            if pending {
                return Ok(false);
            }
            previous_digit = false;
        }
    }
    Ok(!pending)
}

fn hexadecimal(input: &[u8], sign: usize, work: Work<'_>) -> Result<ParsedFloat, Interrupted> {
    let negative = input.first() == Some(&b'-');
    let start = sign + 2;
    let mut position = start;
    let mut mantissa = 0u64;
    let mut stored = 0;
    let mut digits = 0i128;
    let mut point = 0i128;
    let mut saw_digits = false;
    let mut saw_dot = false;
    let mut underscores = false;
    let mut truncated = false;
    while let Some(&byte) = input.get(position) {
        work.step(1)?;
        if byte == b'_' {
            underscores = true;
        } else if byte == b'.' {
            if saw_dot {
                break;
            }
            saw_dot = true;
            point = digits;
        } else {
            let value = digit(byte);
            if value == 16 {
                break;
            }
            saw_digits = true;
            if value == 0 && digits == 0 {
                point -= 1;
            } else {
                digits += 1;
                if stored < 16 {
                    mantissa = (mantissa << 4) | u64::from(value);
                    stored += 1;
                } else if value != 0 {
                    truncated = true;
                }
            }
        }
        position += 1;
    }
    if !saw_digits {
        return Ok(ParsedFloat::syntax());
    }
    if !saw_dot {
        point = digits;
    }
    // Wide positions keep the fixed-state scanner independent of input length.
    point *= 4;
    if !matches!(input.get(position), Some(b'p' | b'P')) {
        return Ok(ParsedFloat::syntax());
    }
    position += 1;
    let negative_exponent = input.get(position) == Some(&b'-');
    if matches!(input.get(position), Some(b'+' | b'-')) {
        position += 1;
    }
    if !input.get(position).is_some_and(u8::is_ascii_digit) {
        return Ok(ParsedFloat::syntax());
    }
    let mut exponent = 0;
    while let Some(&byte) = input.get(position) {
        work.step(1)?;
        if byte == b'_' {
            underscores = true;
        } else if byte.is_ascii_digit() {
            if exponent < 10000 {
                exponent = exponent * 10 + i128::from(byte - b'0');
            }
        } else {
            break;
        }
        position += 1;
    }
    if position != input.len() || (underscores && !underscore_ok(input, start, work)?) {
        return Ok(ParsedFloat::syntax());
    }
    point += if negative_exponent {
        -exponent
    } else {
        exponent
    };
    let mut exponent = if mantissa == 0 { 0 } else { point - stored * 4 };
    exponent += 52;
    while mantissa != 0 && mantissa >> 54 == 0 {
        work.step(1)?;
        mantissa <<= 1;
        exponent -= 1;
    }
    if truncated {
        mantissa |= 1;
    }
    while mantissa >> 55 != 0 {
        work.step(1)?;
        mantissa = (mantissa >> 1) | (mantissa & 1);
        exponent += 1;
    }
    while mantissa > 1 && exponent < -1024 {
        work.step(1)?;
        mantissa = (mantissa >> 1) | (mantissa & 1);
        exponent += 1;
    }
    // Keep the sticky bit until the single final ties-to-even rounding.
    let mut round = mantissa & 3;
    mantissa >>= 2;
    round |= mantissa & 1;
    exponent += 2;
    if round == 3 {
        mantissa += 1;
        if mantissa == 1 << 53 {
            mantissa >>= 1;
            exponent += 1;
        }
    }
    if mantissa >> 52 == 0 {
        exponent = -1023;
    }
    let error = if exponent > 1023 {
        mantissa = 1 << 52;
        exponent = 1024;
        Some(FloatError::Range)
    } else {
        None
    };
    let bits = (mantissa & ((1 << 52) - 1))
        | ((((exponent + 1023) as u64) & 0x7ff) << 52)
        | (u64::from(negative) << 63);
    Ok(ParsedFloat {
        value: f64::from_bits(bits),
        error,
    })
}

/// Float-only conversion for labels and NUMBER's fallback; syntax failure retains zero.
pub(crate) fn parse_checked(
    input: &[u8],
    context: Option<&AllocationContext>,
    account: Option<&QueryAccounting>,
    control: Option<&Control>,
) -> Result<ParsedFloat, Interrupted> {
    let work = Work {
        context,
        account,
        control,
    };
    work.step(0)?;
    let special = if input.eq_ignore_ascii_case(b"nan") {
        Some(f64::from_bits(0x7ff8_0000_0000_0001))
    } else if [b"inf".as_slice(), b"infinity", b"+inf", b"+infinity"]
        .iter()
        .any(|name| input.eq_ignore_ascii_case(name))
    {
        Some(f64::INFINITY)
    } else if [b"-inf".as_slice(), b"-infinity"]
        .iter()
        .any(|name| input.eq_ignore_ascii_case(name))
    {
        Some(f64::NEG_INFINITY)
    } else {
        None
    };
    if let Some(value) = special {
        return Ok(ParsedFloat { value, error: None });
    }
    let sign = usize::from(matches!(input.first(), Some(b'+' | b'-')));
    if input.get(sign) == Some(&b'0') && matches!(input.get(sign + 1), Some(b'x' | b'X')) {
        return hexadecimal(input, sign, work);
    }
    let parsed = decimal(input, sign, work)?;
    work.step(0)?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{EngineAccounting, MemoryKind};

    #[test]
    fn decimal_source_exponent_prefix_freezing_preserves_cancellation() {
        // Original Go readFloat stops accumulating after the exponent reaches 10000.
        let input = [b"1".as_slice(), &vec![b'0'; 10000], b"e-100000"].concat();
        let parsed = parse_checked(&input, None, None, None).unwrap();
        assert_eq!(
            (parsed.value.to_bits(), parsed.error),
            (0x3ff0_0000_0000_0000, None)
        );
    }

    #[test]
    fn decimal_source_fallback_retains_its_distinct_decimal_point() {
        // Original decimal.set's no-dot point uses retained digits, unlike readFloat.
        let input = [
            b"10000000000000001111".as_slice(),
            &vec![b'0'; 781],
            b"e-800",
        ]
        .concat();
        let parsed = parse_checked(&input, None, None, None).unwrap();
        assert_eq!(
            (parsed.value.to_bits(), parsed.error),
            (0x3fb9_9999_9999_999a, None)
        );
    }

    #[test]
    fn float_only_labels_preserve_zero_and_do_not_use_integer_radices() {
        for (input, bits, error) in [
            (b"010".as_slice(), 0x4024_0000_0000_0000, None),
            (b"-0", 0x8000_0000_0000_0000, None),
            (b"0x10", 0, Some(FloatError::Syntax)),
            (b"0x_1p0", 0x3ff0_0000_0000_0000, None),
            (b"+NaN", 0, Some(FloatError::Syntax)),
            (b"NaN", 0x7ff8_0000_0000_0001, None),
            (b"-Inf", 0xfff0_0000_0000_0000, None),
            (b"0x1p1024", 0x7ff0_0000_0000_0000, Some(FloatError::Range)),
            (b"0x1p1024x", 0, Some(FloatError::Syntax)),
            (b"1e99999x", 0, Some(FloatError::Syntax)),
            (b" 1", 0, Some(FloatError::Syntax)),
            (b"1\x00", 0, Some(FloatError::Syntax)),
            (b"1\xff", 0, Some(FloatError::Syntax)),
        ] {
            let parsed = parse_checked(input, None, None, None).unwrap();
            assert_eq!(
                (parsed.value.to_bits(), parsed.error),
                (bits, error),
                "{input:?}"
            );
        }
    }

    #[test]
    fn hexadecimal_rounding_retains_subnormal_and_late_sticky_bits() {
        for (input, bits) in [
            (b"0x1.00000000000008p0".as_slice(), 0x3ff0_0000_0000_0000),
            (b"0x1.000000000000081p0", 0x3ff0_0000_0000_0001),
            (b"0x1.00000000000018p0", 0x3ff0_0000_0000_0002),
            (b"0x0.00000000000008p-1022", 0),
            (b"-0x0.00000000000008p-1022", 0x8000_0000_0000_0000),
            (b"0x0.000000000000081p-1022", 1),
            (b"0x0.00000000000058p-1022", 6),
            (b"0x1.fffffffffffff7fffp1023", 0x7fef_ffff_ffff_ffff),
        ] {
            let parsed = parse_checked(input, None, None, None).unwrap();
            assert_eq!(
                (parsed.value.to_bits(), parsed.error),
                (bits, None),
                "{input:?}"
            );
        }
        let input = [b"0x1.00000000000008".as_slice(), &vec![b'0'; 10000], b"1p0"].concat();
        let parsed = parse_checked(&input, None, None, None).unwrap();
        assert_eq!(parsed.value.to_bits(), 0x3ff0_0000_0000_0001);
        assert_eq!(parsed.error, None);
    }

    #[test]
    fn decimal_scalar_scan_borrows_input_without_heap_scratch() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let input = [b"1.".as_slice(), &vec![b'0'; 4096]].concat();
        let parsed = parse_checked(
            &input,
            Some(&context),
            Some(&account),
            Some(&Control::default()),
        )
        .unwrap();
        assert_eq!(parsed.value, 1.0);
        assert_eq!(parsed.error, None);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].peak,
            0
        );
        assert!(account.snapshot().work >= input.len() as u64);
        account.snapshot().assert_memory_released();
        drop(context);
        drop(account);
        engine.snapshot().assert_memory_released();
    }

    #[test]
    fn decimal_source_fallback_scratch_is_charged_and_released() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Workspace);
        let input = [
            b"10000000000000001111".as_slice(),
            &vec![b'0'; 781],
            b"e-800",
        ]
        .concat();
        let parsed = parse_checked(
            &input,
            Some(&context),
            Some(&account),
            Some(&Control::default()),
        )
        .unwrap();
        assert_eq!(parsed.value.to_bits(), 0x3fb9_9999_9999_999a);
        assert_eq!(parsed.error, None);
        assert_eq!(
            account.snapshot().memory[MemoryKind::Workspace as usize].peak,
            800
        );
        assert!(account.snapshot().work > input.len() as u64);
        account.snapshot().assert_memory_released();
        drop(context);
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
