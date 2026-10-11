// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Prometheus stringlabels and cespare/xxhash v2.3.0; see NOTICE.
use crate::{
    accounting::QueryAccounting,
    evaluator::EvaluationError,
    result::OwnedLabels,
    storage::{Control, Interrupted},
};

const P1: u64 = 11_400_714_785_074_694_791;
const P2: u64 = 14_029_467_366_897_019_727;
const P3: u64 = 1_609_587_929_392_839_161;
const P4: u64 = 9_650_029_242_287_828_579;
const P5: u64 = 2_870_177_450_012_600_261;
fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}
fn merge(acc: u64, input: u64) -> u64 {
    (acc ^ round(0, input)).wrapping_mul(P1).wrapping_add(P4)
}
struct Digest {
    lanes: [u64; 4],
    total: u64,
    buffer: [u8; 32],
    used: usize,
}
impl Digest {
    fn new() -> Self {
        Self {
            lanes: [P1.wrapping_add(P2), P2, 0, 0u64.wrapping_sub(P1)],
            total: 0,
            buffer: [0; 32],
            used: 0,
        }
    }
    fn block(&mut self, bytes: &[u8]) {
        for (lane, chunk) in self.lanes.iter_mut().zip(bytes.chunks_exact(8)) {
            *lane = round(
                *lane,
                u64::from_le_bytes(chunk.try_into().expect("eight-byte lane")),
            );
        }
    }
    fn write(&mut self, mut bytes: &[u8]) {
        self.total = self.total.wrapping_add(bytes.len() as u64);
        if self.used + bytes.len() < 32 {
            self.buffer[self.used..self.used + bytes.len()].copy_from_slice(bytes);
            self.used += bytes.len();
            return;
        }
        if self.used != 0 {
            let needed = 32 - self.used;
            self.buffer[self.used..].copy_from_slice(&bytes[..needed]);
            let block = self.buffer;
            self.block(&block);
            bytes = &bytes[needed..];
            self.used = 0;
        }
        while bytes.len() >= 32 {
            self.block(&bytes[..32]);
            bytes = &bytes[32..];
        }
        self.buffer[..bytes.len()].copy_from_slice(bytes);
        self.used = bytes.len();
    }
    fn finish(&self) -> u64 {
        let mut h = if self.total >= 32 {
            let [a, b, c, d] = self.lanes;
            let mut h = a
                .rotate_left(1)
                .wrapping_add(b.rotate_left(7))
                .wrapping_add(c.rotate_left(12))
                .wrapping_add(d.rotate_left(18));
            for lane in self.lanes {
                h = merge(h, lane);
            }
            h
        } else {
            P5
        };
        h = h.wrapping_add(self.total);
        let mut bytes = &self.buffer[..self.used];
        while bytes.len() >= 8 {
            h ^= round(
                0,
                u64::from_le_bytes(bytes[..8].try_into().expect("eight-byte tail")),
            );
            h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            h ^= (u32::from_le_bytes(bytes[..4].try_into().expect("four-byte tail")) as u64)
                .wrapping_mul(P1);
            h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
            bytes = &bytes[4..];
        }
        for byte in bytes {
            h ^= (*byte as u64).wrapping_mul(P5);
            h = h.rotate_left(11).wrapping_mul(P1);
        }
        h ^= h >> 33;
        h = h.wrapping_mul(P2);
        h ^= h >> 29;
        h = h.wrapping_mul(P3);
        h ^ (h >> 32)
    }
    fn field(
        &mut self,
        bytes: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), Interrupted> {
        for chunk in bytes.chunks(32 * 1024) {
            control.check()?;
            account.work(chunk.len() as u64);
            self.write(chunk);
        }
        Ok(())
    }
    fn sized_field(
        &mut self,
        bytes: &[u8],
        control: &Control,
        account: &QueryAccounting,
    ) -> Result<(), EvaluationError> {
        crate::label_encoding::field(bytes, control, account)?;
        let n = bytes.len();
        if n < 255 {
            self.write(&[n as u8]);
        } else {
            self.write(&[255, n as u8, (n >> 8) as u8, (n >> 16) as u8]);
        }
        self.field(bytes, control, account)?;
        Ok(())
    }
}
pub(crate) fn grouping(
    labels: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<u64, Interrupted> {
    let mut digest = Digest::new();
    for label in labels.0.iter() {
        digest.field(&label.name, control, account)?;
        digest.write(&[255]);
        digest.field(&label.value, control, account)?;
        digest.write(&[255]);
    }
    Ok(digest.finish())
}
pub(crate) fn full(
    labels: &OwnedLabels,
    control: &Control,
    account: &QueryAccounting,
) -> Result<u64, EvaluationError> {
    let mut digest = Digest::new();
    for label in labels.0.iter() {
        digest.sized_field(&label.name, control, account)?;
        digest.sized_field(&label.value, control, account)?;
    }
    Ok(digest.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_zero_seed_digest_vectors_keep_every_chunk_partition() {
        // Pinned xxhash_test.go TestAll zero-seed assertions.
        for (text, expected) in [
            ("", 0xef46db3751d8e999),
            ("a", 0xd24ec4f1a98c6e5b),
            ("as", 0x1c330fb2d66be179),
            ("asd", 0x631c37ce72a97393),
            ("asdf", 0x415872f599cea71e),
            (
                "Call me Ishmael. Some years ago--never mind how long precisely-",
                0x02a2e85470d6fd96,
            ),
        ] {
            for chunk in 1..=text.len().max(1) {
                let mut digest = Digest::new();
                for bytes in text.as_bytes().chunks(chunk) {
                    digest.write(bytes);
                }
                assert_eq!(digest.finish(), expected, "{text:?}, chunk={chunk}");
            }
        }
    }
}
