// SPDX-License-Identifier: GPL-3.0-or-later
use crate::histogram::{FloatHistogram, FloatHistogramRef, IntegerHistogram, IntegerHistogramRef};

pub const STALE_NAN_BITS: u64 = 0x7ff0_0000_0000_0002;

#[derive(Debug)]
pub enum SampleValue {
    Float(f64),
    IntegerHistogram(IntegerHistogram),
    FloatHistogram(FloatHistogram),
}

#[derive(Clone, Copy, Debug)]
pub enum SampleValueRef<'a> {
    Float(f64),
    IntegerHistogram(IntegerHistogramRef<'a>),
    FloatHistogram(FloatHistogramRef<'a>),
}

#[derive(Debug)]
pub struct Sample {
    pub time_ms: i64,
    pub start_time_ms: i64,
    pub value: SampleValue,
}

#[derive(Clone, Copy, Debug)]
pub struct SampleRef<'a> {
    pub time_ms: i64,
    pub start_time_ms: i64,
    pub value: SampleValueRef<'a>,
}

impl Sample {
    pub fn as_ref(&self) -> SampleRef<'_> {
        SampleRef {
            time_ms: self.time_ms,
            start_time_ms: self.start_time_ms,
            value: match &self.value {
                SampleValue::Float(value) => SampleValueRef::Float(*value),
                SampleValue::IntegerHistogram(value) => {
                    SampleValueRef::IntegerHistogram(value.as_ref())
                }
                SampleValue::FloatHistogram(value) => {
                    SampleValueRef::FloatHistogram(value.as_ref())
                }
            },
        }
    }
}

impl SampleValueRef<'_> {
    pub fn is_stale(self) -> bool {
        match self {
            Self::Float(value) => value.to_bits() == STALE_NAN_BITS,
            Self::IntegerHistogram(value) => value.sum.to_bits() == STALE_NAN_BITS,
            Self::FloatHistogram(value) => value.sum.to_bits() == STALE_NAN_BITS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_payload_is_distinct_from_ordinary_nan_in_every_sample_kind() {
        for bits in [
            STALE_NAN_BITS,
            0x7ff8_0000_0000_0001,
            0xfff8_0000_0000_0001,
            0x7ff0_0000_0000_0000,
        ] {
            let value = f64::from_bits(bits);
            let float = FloatHistogram {
                sum: value,
                ..FloatHistogram::default()
            };
            let integer = IntegerHistogram {
                sum: value,
                ..IntegerHistogram::default()
            };
            for sample in [
                SampleValueRef::Float(value),
                SampleValueRef::FloatHistogram(float.as_ref()),
                SampleValueRef::IntegerHistogram(integer.as_ref()),
            ] {
                assert_eq!(sample.is_stale(), bits == STALE_NAN_BITS);
            }
        }
    }
}
