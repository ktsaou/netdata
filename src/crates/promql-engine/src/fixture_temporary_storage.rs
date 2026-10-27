// SPDX-License-Identifier: GPL-3.0-or-later
//! Explicit qualification-provider backing; absent from the production core.
use crate::storage::{Control, StorageError, TemporaryFloatBits, TemporaryFloatBitsRequest};

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
))]
mod disk {
    use super::*;
    use crate::{
        accounting::{AccountedVec, MemoryKind, QueryAccounting},
        storage::{ProviderResources, temporary_storage_error},
    };
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
        os::unix::fs::OpenOptionsExt,
        path::Path,
    };

    // glibc target fcntl headers define __O_TMPFILE | __O_DIRECTORY.
    #[cfg(target_arch = "x86_64")]
    const O_TMPFILE: i32 = 0o20200000;
    #[cfg(any(target_arch = "aarch64", target_arch = "arm"))]
    const O_TMPFILE: i32 = 0o20040000;
    const DIRECTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/promql-conformance/.tmp");

    fn error(
        request: TemporaryFloatBitsRequest<'_>,
        phase: &str,
        cause: std::io::Error,
    ) -> StorageError {
        use std::fmt::Write;
        let mut message = AccountedVec::new(request.accounting, MemoryKind::Workspace);
        write!(message, "temporary float storage {phase}: {cause}")
            .expect("accounted storage error");
        StorageError::Provider(message)
    }
    pub struct FileStore {
        file: File,
        count: u64,
        account: QueryAccounting,
    }
    impl FileStore {
        fn request(&self) -> TemporaryFloatBitsRequest<'_> {
            TemporaryFloatBitsRequest {
                value_count: self.count,
                accounting: &self.account,
            }
        }

        fn seek(&mut self, offset: u64, len: usize) -> Result<usize, StorageError> {
            let count = u64::try_from(len)
                .map_err(|_| StorageError::Invalid("temporary transfer overflow"))?;
            if offset.checked_add(count).is_none_or(|end| end > self.count) {
                return Err(StorageError::Invalid("temporary transfer out of bounds"));
            }
            let bytes = len
                .checked_mul(8)
                .ok_or(StorageError::Invalid("temporary transfer overflow"))?;
            let position = offset
                .checked_mul(8)
                .ok_or(StorageError::Invalid("temporary transfer offset overflow"))?;
            self.file
                .seek(SeekFrom::Start(position))
                .map_err(|cause| error(self.request(), "seek", cause))?;
            Ok(bytes)
        }
    }
    impl TemporaryFloatBits for FileStore {
        fn resources(&self) -> ProviderResources {
            ProviderResources::default()
        }
        fn read_into(
            &mut self,
            offset: u64,
            values: &mut [u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            let len = self.seek(offset, values.len())?;
            // Every u64 bit pattern is valid; the writable byte view stays within this exclusive slice.
            let bytes =
                unsafe { std::slice::from_raw_parts_mut(values.as_mut_ptr().cast::<u8>(), len) };
            self.file
                .read_exact(bytes)
                .map_err(|cause| error(self.request(), "read", cause))
        }
        fn write_at(
            &mut self,
            offset: u64,
            values: &[u64],
            control: &Control,
        ) -> Result<(), StorageError> {
            control.check()?;
            let len = self.seek(offset, values.len())?;
            // Preserve native float bits without a second conversion buffer.
            let bytes = unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), len) };
            self.file
                .write_all(bytes)
                .map_err(|cause| error(self.request(), "write", cause))
        }
    }
    impl FileStore {
        pub fn create(
            request: TemporaryFloatBitsRequest<'_>,
            control: &Control,
        ) -> Result<Self, StorageError> {
            control.check()?;
            let bytes = request.extent_bytes()?;
            let directory = Path::new(DIRECTORY);
            if !directory.is_absolute() || directory.components().count() < 4 || !directory.is_dir()
            {
                return Err(temporary_storage_error(
                    request.accounting,
                    b"temporary float storage fixture directory unavailable",
                ));
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(O_TMPFILE)
                .mode(0o600)
                .open(directory)
                .map_err(|cause| error(request, "create", cause))?;
            file.set_len(bytes)
                .map_err(|cause| error(request, "extent", cause))?;
            Ok(Self {
                file,
                count: request.value_count,
                account: request.accounting.clone(),
            })
        }
    }
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
))]
pub use disk::FileStore;

pub fn open_store(
    request: TemporaryFloatBitsRequest<'_>,
    control: &Control,
) -> Result<Option<crate::storage::OwnedTemporary<'static>>, StorageError> {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
    ))]
    {
        Ok(Some(crate::storage::OwnedTemporary::new(
            FileStore::create(request, control)?,
            request.accounting,
        )))
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
    )))]
    {
        control.check()?;
        request.extent_bytes()?;
        Ok(None)
    }
}

pub fn with_store<R>(
    request: TemporaryFloatBitsRequest<'_>,
    control: &Control,
    visit: impl for<'a> FnOnce(&'a mut dyn TemporaryFloatBits) -> R,
) -> Result<R, StorageError> {
    #[cfg(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
    ))]
    {
        let mut store = FileStore::create(request, control)?;
        Ok(visit(&mut store))
    }
    #[cfg(not(all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "arm")
    )))]
    {
        let _ = (control, visit);
        Err(crate::storage::temporary_storage_error(
            request.accounting,
            b"temporary float storage fixture backend unsupported",
        ))
    }
}
