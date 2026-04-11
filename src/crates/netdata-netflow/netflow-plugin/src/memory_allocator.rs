#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AllocatorMemorySample {
    pub(crate) heap_in_use_bytes: u64,
    pub(crate) heap_free_bytes: u64,
    pub(crate) heap_arena_bytes: u64,
    pub(crate) mmap_in_use_bytes: u64,
    pub(crate) releasable_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AllocatorTrimResult {
    pub(crate) before: AllocatorMemorySample,
    pub(crate) after: AllocatorMemorySample,
}

const DEFAULT_TRIM_THRESHOLD_BYTES: u64 = 64 * 1024 * 1024;
const PR_SET_THP_DISABLE: libc::c_int = 41;
const NETFLOW_GLIBC_ARENA_MAX: libc::c_int = 1;

pub(crate) fn current_allocator_memory() -> AllocatorMemorySample {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        let sample = libc::mallinfo2();
        return AllocatorMemorySample {
            heap_in_use_bytes: sample.uordblks as u64,
            heap_free_bytes: sample.fordblks as u64,
            heap_arena_bytes: sample.arena as u64,
            mmap_in_use_bytes: sample.hblkhd as u64,
            releasable_bytes: sample.keepcost as u64,
        };
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        AllocatorMemorySample::default()
    }
}

pub(crate) fn trim_allocator_if_worthwhile() -> Option<AllocatorTrimResult> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let before = current_allocator_memory();
        if before.heap_free_bytes < DEFAULT_TRIM_THRESHOLD_BYTES
            || before.heap_free_bytes <= before.heap_in_use_bytes
        {
            return None;
        }

        unsafe {
            libc::malloc_trim(0);
        }

        let after = current_allocator_memory();
        return Some(AllocatorTrimResult { before, after });
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

pub(crate) fn limit_glibc_arenas_for_process() -> Option<libc::c_int> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        if libc::mallopt(libc::M_ARENA_MAX, NETFLOW_GLIBC_ARENA_MAX) == 1 {
            return Some(NETFLOW_GLIBC_ARENA_MAX);
        }

        return None;
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

pub(crate) fn disable_transparent_huge_pages_for_process() -> bool {
    #[cfg(target_os = "linux")]
    unsafe {
        return libc::prctl(PR_SET_THP_DISABLE, 1, 0, 0, 0) == 0;
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}
