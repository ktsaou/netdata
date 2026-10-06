// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    accounting::{AccountedVec, AllocationContext, QueryAccounting},
    storage::{Control, Interrupted},
};
use std::fmt::{self, Write};

/// Borrow the compile owner; buffers retain their own leases after this scope ends.
#[derive(Clone, Copy, Default)]
pub(super) struct Work<'a> {
    pub(super) context: Option<&'a AllocationContext>,
    pub(super) account: Option<&'a QueryAccounting>,
    pub(super) control: Option<&'a Control>,
}
impl Work<'_> {
    pub(super) fn step(self, units: usize) -> Result<(), Interrupted> {
        if let Some(control) = self.control {
            control.check()?;
        }
        if let Some(account) = self.account {
            account.work(units as u64);
        }
        Ok(())
    }
    pub(super) fn buffer<T>(self) -> AccountedVec<T> {
        AccountedVec::new_in(self.context)
    }
    pub(super) fn copy(self, bytes: &[u8]) -> Result<AccountedVec<u8>, Interrupted> {
        self.step(0)?;
        let mut result = AccountedVec::with_capacity_in(bytes.len(), self.context);
        self.extend(&mut result, bytes)?;
        Ok(result)
    }
    pub(super) fn extend(
        self,
        target: &mut AccountedVec<u8>,
        bytes: &[u8],
    ) -> Result<(), Interrupted> {
        if let (Some(account), Some(control)) = (self.account, self.control) {
            return crate::result::append_bytes_checked(target, bytes, account, control);
        }
        for chunk in bytes.chunks(1024) {
            self.step(chunk.len())?;
            target.extend_from_slice(chunk);
        }
        Ok(())
    }
    pub(super) fn contains(self, bytes: &[u8], byte: u8) -> Result<bool, Interrupted> {
        for chunk in bytes.chunks(1024) {
            self.step(chunk.len())?;
            if chunk.contains(&byte) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn equal(self, left: &[u8], right: &[u8]) -> Result<bool, Interrupted> {
        if left.len() != right.len() {
            return Ok(false);
        }
        for (left, right) in left.chunks(1024).zip(right.chunks(1024)) {
            self.step(left.len())?;
            if left != right {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn valid_utf8(self, mut bytes: &[u8]) -> Result<bool, Interrupted> {
        while !bytes.is_empty() {
            let (rune, width) = super::quote::decode_rune(bytes);
            self.step(width)?;
            if rune == 0xfffd && width == 1 && bytes[0] >= 0x80 {
                return Ok(false);
            }
            bytes = &bytes[width..];
        }
        Ok(true)
    }
    pub(super) fn format(
        self,
        arguments: fmt::Arguments<'_>,
    ) -> Result<AccountedVec<u8>, Interrupted> {
        let mut writer = Writer {
            work: self,
            bytes: self.buffer(),
            interrupted: None,
        };
        let result = writer.write_fmt(arguments);
        if let Some(error) = writer.interrupted {
            return Err(error);
        }
        result.expect("frontend byte writer");
        Ok(writer.bytes)
    }
}

struct Writer<'a> {
    work: Work<'a>,
    bytes: AccountedVec<u8>,
    interrupted: Option<Interrupted>,
}
impl Write for Writer<'_> {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.interrupted.is_some() {
            return Err(fmt::Error);
        }
        if let Err(error) = self.work.extend(&mut self.bytes, value.as_bytes()) {
            self.interrupted = Some(error);
            return Err(fmt::Error);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        accounting::{EngineAccounting, MemoryKind},
        frontend::{lexer::Lexer, literal, number, quote},
        test_storage::interrupt_during,
    };

    fn operation(work: Work<'_>, name: &str, bytes: &[u8]) -> Result<(), Interrupted> {
        match name {
            "copy" => work.copy(bytes).map(|_| ()),
            "format" => work
                .format(format_args!("{}", quote::quoted(bytes)))
                .map(|_| ()),
            "compare" => work.equal(bytes, bytes).map(|_| ()),
            "contains" => work.contains(bytes, b'\n').map(|_| ()),
            "utf8" => work.valid_utf8(bytes).map(|_| ()),
            "literal" | "escape" => literal::unquote_in(bytes, work).map(|_| ()),
            "number" => number::number_in(bytes, work).map(|_| ()),
            "duration" => number::duration_in(bytes, work).map(|_| ()),
            "lexer" => Lexer::new_in(bytes, false, work).next_in().map(|_| ()),
            _ => unreachable!(),
        }
    }

    #[test]
    fn long_frontend_operations_return_cancellation_from_owned_work() {
        for name in [
            "copy", "format", "compare", "contains", "utf8", "literal", "escape", "number",
            "duration", "lexer",
        ] {
            let bytes = match name {
                "literal" => [b"\"".as_slice(), &vec![b'a'; 400_000], b"\""].concat(),
                "escape" => [b"\"".as_slice(), &b"\\n".repeat(200_000), b"\""].concat(),
                "number" => vec![b'1'; 400_000],
                "duration" => [&vec![b'0'; 400_000], b"h".as_slice()].concat(),
                _ => vec![b'a'; 400_000],
            };
            let engine = EngineAccounting::default();
            let account = engine.query();
            let context = AllocationContext::new(&account, MemoryKind::Compiled);
            let control = Control::default();
            let work = Work {
                context: Some(&context),
                account: Some(&account),
                control: Some(&control),
            };
            let result = interrupt_during(&account, &control, || operation(work, name, &bytes));
            assert_eq!(result, Err(Interrupted::Cancelled), "{name}");
            assert!(account.snapshot().work >= 8192, "{name}");
            account.snapshot().assert_memory_released();
            drop(context);
            drop(account);
            engine.snapshot().assert_memory_released();
        }
    }

    #[test]
    fn frontend_deadlines_remain_distinct_from_syntax_errors() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let context = AllocationContext::new(&account, MemoryKind::Compiled);
        let control = Control {
            deadline: Some(std::time::Instant::now()),
            ..Control::default()
        };
        let work = Work {
            context: Some(&context),
            account: Some(&account),
            control: Some(&control),
        };
        for (name, input) in [
            ("copy", b"abc".as_slice()),
            ("format", b"abc"),
            ("compare", b"abc"),
            ("contains", b"abc"),
            ("utf8", b"abc"),
            ("literal", b"\"abc\""),
            ("escape", b"\"\\n\""),
            ("number", b"123"),
            ("duration", b"123h"),
            ("lexer", b"abc"),
        ] {
            assert_eq!(
                operation(work, name, input),
                Err(Interrupted::Deadline),
                "{name}"
            );
        }
        account.snapshot().assert_memory_released();
        drop(context);
        drop(account);
        engine.snapshot().assert_memory_released();
    }
}
