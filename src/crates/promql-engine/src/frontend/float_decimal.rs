// SPDX-License-Identifier: GPL-3.0-or-later
// Go 1.27.1 internal/strconv/atof.go and decimal.go; see NOTICE.
use super::{
    float::{FloatError, ParsedFloat},
    work::Work,
};
use crate::{accounting::AccountedVec, storage::Interrupted};

const DIGITS: usize = 800;
const MAX_SHIFT: i32 = usize::BITS as i32 - 4;

struct Decimal {
    digits: AccountedVec<u8>,
    used: usize,
    point: i128,
    truncated: bool,
}

impl Decimal {
    fn read(mantissa: &[u8], exponent: i128, work: Work<'_>) -> Result<Self, Interrupted> {
        work.step(DIGITS)?;
        let mut digits = AccountedVec::with_capacity_in(DIGITS, work.context);
        digits.resize(DIGITS, 0);
        let mut value = Self {
            digits,
            used: 0,
            point: 0,
            truncated: false,
        };
        let mut dot = false;
        for &byte in mantissa {
            work.step(1)?;
            match byte {
                b'_' => {}
                b'.' => {
                    dot = true;
                    value.point = value.used as i128;
                }
                b'0' if value.used == 0 => value.point -= 1,
                _ => {
                    if value.used < DIGITS {
                        value.digits[value.used] = byte;
                        value.used += 1;
                    } else if byte != b'0' {
                        value.truncated = true;
                    }
                }
            }
        }
        // The fallback point deliberately counts retained digits when there is no dot.
        if !dot {
            value.point = value.used as i128;
        }
        value.point += exponent;
        Ok(value)
    }

    fn trim(&mut self, work: Work<'_>) -> Result<(), Interrupted> {
        while self.used > 0 && self.digits[self.used - 1] == b'0' {
            work.step(1)?;
            self.used -= 1;
        }
        if self.used == 0 {
            self.point = 0;
        }
        Ok(())
    }

    fn right_shift(&mut self, shift: u32, work: Work<'_>) -> Result<(), Interrupted> {
        let mut read = 0;
        let mut write = 0;
        let mut value = 0u64;
        while value >> shift == 0 {
            work.step(1)?;
            if read >= self.used {
                if value == 0 {
                    self.used = 0;
                    return Ok(());
                }
                while value >> shift == 0 {
                    work.step(1)?;
                    value *= 10;
                    read += 1;
                }
                break;
            }
            value = value * 10 + u64::from(self.digits[read] - b'0');
            read += 1;
        }
        self.point -= read as i128 - 1;
        let mask = (1u64 << shift) - 1;
        while read < self.used {
            work.step(1)?;
            let digit = self.digits[read];
            self.digits[write] = (value >> shift) as u8 + b'0';
            write += 1;
            value = (value & mask) * 10 + u64::from(digit - b'0');
            read += 1;
        }
        while value > 0 {
            work.step(1)?;
            let digit = value >> shift;
            value &= mask;
            if write < DIGITS {
                self.digits[write] = digit as u8 + b'0';
                write += 1;
            } else if digit > 0 {
                self.truncated = true;
            }
            value *= 10;
        }
        self.used = write;
        self.trim(work)
    }

    fn prefix_less(&self, cutoff: &[u8], work: Work<'_>) -> Result<bool, Interrupted> {
        for (index, &byte) in cutoff.iter().enumerate() {
            work.step(1)?;
            if index >= self.used {
                return Ok(true);
            }
            if self.digits[index] != byte {
                return Ok(self.digits[index] < byte);
            }
        }
        Ok(false)
    }

    fn left_shift(&mut self, shift: u32, work: Work<'_>) -> Result<(), Interrupted> {
        let (mut delta, cutoff) = LEFT_CHEATS[shift as usize];
        if self.prefix_less(cutoff, work)? {
            delta -= 1;
        }
        let mut read = self.used;
        let mut write = self.used + delta;
        let mut value = 0u64;
        while read > 0 {
            work.step(1)?;
            read -= 1;
            value += u64::from(self.digits[read] - b'0') << shift;
            let quotient = value / 10;
            let remainder = value - 10 * quotient;
            write -= 1;
            if write < DIGITS {
                self.digits[write] = remainder as u8 + b'0';
            } else if remainder != 0 {
                self.truncated = true;
            }
            value = quotient;
        }
        while value > 0 {
            work.step(1)?;
            let quotient = value / 10;
            let remainder = value - 10 * quotient;
            write -= 1;
            if write < DIGITS {
                self.digits[write] = remainder as u8 + b'0';
            } else if remainder != 0 {
                self.truncated = true;
            }
            value = quotient;
        }
        self.used = (self.used + delta).min(DIGITS);
        self.point += delta as i128;
        self.trim(work)
    }

    fn shift(&mut self, mut shift: i32, work: Work<'_>) -> Result<(), Interrupted> {
        work.step(0)?;
        if self.used == 0 {
            return Ok(());
        }
        if shift > 0 {
            while shift > MAX_SHIFT {
                self.left_shift(MAX_SHIFT as u32, work)?;
                shift -= MAX_SHIFT;
            }
            self.left_shift(shift as u32, work)?;
        } else if shift < 0 {
            while shift < -MAX_SHIFT {
                self.right_shift(MAX_SHIFT as u32, work)?;
                shift += MAX_SHIFT;
            }
            self.right_shift((-shift) as u32, work)?;
        }
        Ok(())
    }

    fn round_up(&self, digits: i128) -> bool {
        if digits < 0 || digits >= self.used as i128 {
            return false;
        }
        let digits = digits as usize;
        if self.digits[digits] == b'5' && digits + 1 == self.used {
            self.truncated || (digits > 0 && !(self.digits[digits - 1] - b'0').is_multiple_of(2))
        } else {
            self.digits[digits] >= b'5'
        }
    }

    fn rounded_integer(&self, work: Work<'_>) -> Result<u64, Interrupted> {
        if self.point > 20 {
            return Ok(u64::MAX);
        }
        let mut index = 0;
        let mut value = 0u64;
        while (index as i128) < self.point && index < self.used {
            work.step(1)?;
            value = value
                .wrapping_mul(10)
                .wrapping_add(u64::from(self.digits[index] - b'0'));
            index += 1;
        }
        while (index as i128) < self.point {
            work.step(1)?;
            value = value.wrapping_mul(10);
            index += 1;
        }
        if self.round_up(self.point) {
            value = value.wrapping_add(1);
        }
        Ok(value)
    }

    fn float(&mut self, sign: u64, work: Work<'_>) -> Result<ParsedFloat, Interrupted> {
        const POWERS: [i32; 9] = [1, 3, 6, 9, 13, 16, 19, 23, 26];
        if self.used == 0 || self.point < -330 {
            return Ok(ParsedFloat {
                value: f64::from_bits(sign),
                error: None,
            });
        }
        if self.point > 310 {
            return Ok(ParsedFloat {
                value: f64::from_bits(sign | (0x7ff << 52)),
                error: Some(FloatError::Range),
            });
        }
        let mut exponent = 0;
        while self.point > 0 {
            let shift = if self.point >= POWERS.len() as i128 {
                27
            } else {
                POWERS[self.point as usize]
            };
            self.shift(-shift, work)?;
            exponent += shift;
        }
        while self.point < 0 || (self.point == 0 && self.digits[0] < b'5') {
            let shift = if -self.point >= POWERS.len() as i128 {
                27
            } else {
                POWERS[(-self.point) as usize]
            };
            self.shift(shift, work)?;
            exponent -= shift;
        }
        exponent -= 1;
        if exponent < -1022 {
            let shift = -1022 - exponent;
            self.shift(-shift, work)?;
            exponent += shift;
        }
        if exponent + 1023 >= 0x7ff {
            return Ok(ParsedFloat {
                value: f64::from_bits(sign | (0x7ff << 52)),
                error: Some(FloatError::Range),
            });
        }
        self.shift(53, work)?;
        let mut mantissa = self.rounded_integer(work)?;
        if mantissa == 2 << 52 {
            mantissa >>= 1;
            exponent += 1;
            if exponent + 1023 >= 0x7ff {
                return Ok(ParsedFloat {
                    value: f64::from_bits(sign | (0x7ff << 52)),
                    error: Some(FloatError::Range),
                });
            }
        }
        if mantissa & (1 << 52) == 0 {
            exponent = -1023;
        }
        Ok(ParsedFloat {
            value: f64::from_bits(
                (mantissa & ((1 << 52) - 1)) | (((exponent + 1023) as u64) << 52) | sign,
            ),
            error: None,
        })
    }
}

pub(super) fn parse(
    mantissa: &[u8],
    exponent: i128,
    sign: u64,
    work: Work<'_>,
) -> Result<ParsedFloat, Interrupted> {
    Decimal::read(mantissa, exponent, work)?.float(sign, work)
}

const LEFT_CHEATS: [(usize, &[u8]); 61] = [
    (0, b""),
    (1, b"5"),
    (1, b"25"),
    (1, b"125"),
    (2, b"625"),
    (2, b"3125"),
    (2, b"15625"),
    (3, b"78125"),
    (3, b"390625"),
    (3, b"1953125"),
    (4, b"9765625"),
    (4, b"48828125"),
    (4, b"244140625"),
    (4, b"1220703125"),
    (5, b"6103515625"),
    (5, b"30517578125"),
    (5, b"152587890625"),
    (6, b"762939453125"),
    (6, b"3814697265625"),
    (6, b"19073486328125"),
    (7, b"95367431640625"),
    (7, b"476837158203125"),
    (7, b"2384185791015625"),
    (7, b"11920928955078125"),
    (8, b"59604644775390625"),
    (8, b"298023223876953125"),
    (8, b"1490116119384765625"),
    (9, b"7450580596923828125"),
    (9, b"37252902984619140625"),
    (9, b"186264514923095703125"),
    (10, b"931322574615478515625"),
    (10, b"4656612873077392578125"),
    (10, b"23283064365386962890625"),
    (10, b"116415321826934814453125"),
    (11, b"582076609134674072265625"),
    (11, b"2910383045673370361328125"),
    (11, b"14551915228366851806640625"),
    (12, b"72759576141834259033203125"),
    (12, b"363797880709171295166015625"),
    (12, b"1818989403545856475830078125"),
    (13, b"9094947017729282379150390625"),
    (13, b"45474735088646411895751953125"),
    (13, b"227373675443232059478759765625"),
    (13, b"1136868377216160297393798828125"),
    (14, b"5684341886080801486968994140625"),
    (14, b"28421709430404007434844970703125"),
    (14, b"142108547152020037174224853515625"),
    (15, b"710542735760100185871124267578125"),
    (15, b"3552713678800500929355621337890625"),
    (15, b"17763568394002504646778106689453125"),
    (16, b"88817841970012523233890533447265625"),
    (16, b"444089209850062616169452667236328125"),
    (16, b"2220446049250313080847263336181640625"),
    (16, b"11102230246251565404236316680908203125"),
    (17, b"55511151231257827021181583404541015625"),
    (17, b"277555756156289135105907917022705078125"),
    (17, b"1387778780781445675529539585113525390625"),
    (18, b"6938893903907228377647697925567626953125"),
    (18, b"34694469519536141888238489627838134765625"),
    (18, b"173472347597680709441192448139190673828125"),
    (19, b"867361737988403547205962240695953369140625"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_decimal_shift_results_preserve_digits_and_point() {
        // Go 1.27.1 decimal_test.go:20-29, including its power-of-two boundary pair.
        for (input, shift, digits, point) in [
            (b"0".as_slice(), -100, b"".as_slice(), 0),
            (b"0", 100, b"", 0),
            (b"1", 100, b"1267650600228229401496703205376", 31),
            (
                b"1",
                -100,
                b"7888609052210118054117285652827862296732064351090230047702789306640625",
                -30,
            ),
            (b"12345678", 8, b"3160493568", 10),
            (b"12345678", -8, b"482253046875", 5),
            (b"195312", 9, b"99999744", 8),
            (b"1953125", 9, b"1", 10),
        ] {
            let work = Work::default();
            let mut value = Decimal::read(input, 0, work).unwrap();
            value.shift(shift, work).unwrap();
            assert_eq!((&value.digits[..value.used], value.point), (digits, point));
            assert!(!value.truncated);
        }
    }

    #[test]
    fn original_decimal_rounded_integer_results_preserve_ties() {
        // Go 1.27.1 decimal_test.go:103-113.
        for (input, shift, integer) in [
            (b"0".as_slice(), 100, 0),
            (b"512", -8, 2),
            (b"513", -8, 2),
            (b"640", -8, 2),
            (b"641", -8, 3),
            (b"384", -8, 2),
            (b"385", -8, 2),
            (b"383", -8, 1),
            (b"1", 100, u64::MAX),
            (b"1000", 0, 1000),
        ] {
            let work = Work::default();
            let mut value = Decimal::read(input, 0, work).unwrap();
            value.shift(shift, work).unwrap();
            assert_eq!(value.rounded_integer(work).unwrap(), integer);
        }
    }
}
