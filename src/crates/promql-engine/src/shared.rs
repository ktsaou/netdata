// SPDX-License-Identifier: GPL-3.0-or-later
//! Strong ownership of one known Box allocation and its requested-byte charge.
use crate::accounting::{AllocationContext, EngineAccounting, SharedCharge};
use std::{
    alloc::Layout,
    fmt,
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering, fence},
};

// The final payload permits ordinary structural Box coercion, including trait metadata.
#[repr(C)]
pub(crate) struct Allocation<T: ?Sized> {
    strong: AtomicUsize,
    charge: Option<SharedCharge>,
    value: T,
}
impl<T> Allocation<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            strong: AtomicUsize::new(1),
            charge: None,
            value,
        }
    }
    pub(crate) fn charge(context: Option<&AllocationContext>) -> Option<SharedCharge> {
        context.map(|context| context.hold(Layout::new::<Self>().size() as u64).into())
    }
}

/// Immutable aliases retain the allocation and its originating accounting context.
pub struct Strong<T: ?Sized> {
    allocation: NonNull<Allocation<T>>,
    owned: PhantomData<Allocation<T>>,
}

// All shared header and payload fields must satisfy both thread contracts.
unsafe impl<T: ?Sized> Send for Strong<T> where Allocation<T>: Send + Sync {}
unsafe impl<T: ?Sized> Sync for Strong<T> where Allocation<T>: Send + Sync {}

// Declaration order keeps the charge until payload destruction and Box deallocation.
struct Pending<T: ?Sized> {
    allocation: Box<Allocation<T>>,
    charge: Option<SharedCharge>,
}
impl<T> Strong<T> {
    pub(crate) fn allocation_bytes() -> u64 {
        Layout::new::<Allocation<T>>().size() as u64
    }
    pub(crate) fn new_in(value: T, context: Option<&AllocationContext>) -> Self {
        let charge = Allocation::<T>::charge(context);
        let allocation = Box::new(Allocation::new(value));
        Self::finish(allocation, charge)
    }
    pub(crate) fn new_for_engine(value: T, engine: &EngineAccounting) -> Self {
        let charge = SharedCharge::Engine(engine.hold_accounting(Self::allocation_bytes()));
        let allocation = Box::new(Allocation::new(value));
        Self::finish(allocation, Some(charge))
    }
}
impl<T: ?Sized> Strong<T> {
    pub(crate) fn finish(allocation: Box<Allocation<T>>, charge: Option<SharedCharge>) -> Self {
        let mut pending = Pending { allocation, charge };
        assert!(pending.allocation.charge.is_none(), "unshared allocation");
        assert_eq!(pending.allocation.strong.load(Ordering::Relaxed), 1);
        if let Some(charge) = &pending.charge {
            assert_eq!(
                Layout::for_value(&*pending.allocation).size() as u64,
                charge.bytes()
            );
        }
        pending.allocation.charge = pending.charge.take();
        // Nothing fallible may intervene between charge installation and ownership transfer.
        let allocation = unsafe { NonNull::new_unchecked(Box::into_raw(pending.allocation)) };
        Self {
            allocation,
            owned: PhantomData,
        }
    }
    pub fn same_allocation(&self, other: &Self) -> bool {
        std::ptr::addr_eq(self.allocation.as_ptr(), other.allocation.as_ptr())
    }
    pub(crate) fn owns(&self, context: &AllocationContext) -> bool {
        // This live alias prevents unique destruction or charge removal.
        unsafe { self.allocation.as_ref() }
            .charge
            .as_ref()
            .is_some_and(|charge| charge.owns(context))
    }
    pub(crate) fn engine_owned(&self) -> bool {
        // The immutable charge records creation ownership, independent of later consumers.
        unsafe { self.allocation.as_ref() }
            .charge
            .as_ref()
            .is_some_and(SharedCharge::engine_owned)
    }
}
fn increment(strong: &AtomicUsize) {
    let mut current = strong.load(Ordering::Relaxed);
    loop {
        assert!(
            current != 0 && current < isize::MAX as usize,
            "strong reference count overflow"
        );
        match strong.compare_exchange_weak(
            current,
            current + 1,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(actual) => current = actual,
        }
    }
}
impl<T: ?Sized> Clone for Strong<T> {
    fn clone(&self) -> Self {
        // The existing owner keeps the Box alive throughout the checked increment.
        increment(&unsafe { self.allocation.as_ref() }.strong);
        Self {
            allocation: self.allocation,
            owned: PhantomData,
        }
    }
}
impl<T: ?Sized> Deref for Strong<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // Shared aliases expose only an immutable payload reference.
        unsafe { &self.allocation.as_ref().value }
    }
}
impl<T: fmt::Debug + ?Sized> fmt::Debug for Strong<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Strong").field(&&**self).finish()
    }
}
impl<T: ?Sized> Drop for Strong<T> {
    fn drop(&mut self) {
        if unsafe { self.allocation.as_ref() }
            .strong
            .fetch_sub(1, Ordering::Release)
            != 1
        {
            return;
        }
        fence(Ordering::Acquire);
        // Only the last owner reconstructs the original Box pointer and its metadata.
        let mut allocation = unsafe { Box::from_raw(self.allocation.as_ptr()) };
        let charge = allocation.charge.take();
        drop(allocation);
        drop(charge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounting::{EngineAccounting, MemoryKind, QueryAccounting};
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        sync::Arc,
    };

    fn context(account: &QueryAccounting) -> AllocationContext {
        AllocationContext::new(account, MemoryKind::Workspace)
    }
    #[test]
    fn aliases_charge_one_layout_until_the_last_owner_outlives_the_query_handle() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let value = Strong::new_in(7u64, Some(&context(&account)));
        let bytes = Layout::new::<Allocation<u64>>().size() as u64;
        assert_eq!(size_of_val(&value), size_of::<usize>());
        assert_eq!(account.snapshot().memory[1].current, bytes);
        let alias = value.clone();
        assert!(value.same_allocation(&alias));
        assert_eq!(account.snapshot().memory[1].acquired, bytes);
        drop(value);
        drop(account);
        assert_eq!(*alias, 7);
        assert_eq!(engine.snapshot().memory[1].current, bytes);
        drop(alias);
        let observation = engine.snapshot().memory[1];
        assert_eq!(observation.current, 0);
        assert_eq!(observation.acquired, observation.released);
    }

    #[derive(Debug)]
    #[repr(align(256))]
    struct Payload {
        account: QueryAccounting,
        drops: Arc<AtomicUsize>,
        panic: bool,
    }
    trait View: fmt::Debug + Send + Sync {
        fn read(&self) -> u64;
    }
    impl View for Payload {
        fn read(&self) -> u64 {
            19
        }
    }
    impl Drop for Payload {
        fn drop(&mut self) {
            assert!(
                self.account.snapshot().memory[1].current
                    >= Layout::new::<Allocation<Self>>().size() as u64
            );
            self.drops.fetch_add(1, Ordering::Relaxed);
            assert!(!self.panic, "synthetic payload drop");
        }
    }
    fn erased(
        account: &QueryAccounting,
        drops: &Arc<AtomicUsize>,
        panic: bool,
    ) -> Strong<dyn View> {
        let charge = Allocation::<Payload>::charge(Some(&context(account)));
        let concrete = Box::new(Allocation::new(Payload {
            account: account.clone(),
            drops: drops.clone(),
            panic,
        }));
        let original = (&*concrete as *const Allocation<Payload>).cast::<()>();
        let allocation: Box<Allocation<dyn View>> = concrete;
        assert_eq!(
            (&*allocation as *const Allocation<dyn View>).cast::<()>(),
            original
        );
        assert_eq!(
            Layout::for_value(&*allocation),
            Layout::new::<Allocation<Payload>>()
        );
        Strong::finish(allocation, charge)
    }
    #[test]
    fn trait_metadata_and_overalignment_survive_aliases_and_panicking_last_drop() {
        for panic in [false, true] {
            let engine = EngineAccounting::default();
            let account = engine.query();
            let drops = Arc::new(AtomicUsize::new(0));
            let value = erased(&account, &drops, panic);
            assert_eq!(size_of_val(&value), 2 * size_of::<usize>());
            let alias = value.clone();
            assert_eq!(alias.read(), 19);
            drop(value);
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            assert_eq!(
                catch_unwind(AssertUnwindSafe(|| drop(alias))).is_err(),
                panic
            );
            assert_eq!(drops.load(Ordering::Relaxed), 1);
            let observation = account.snapshot().memory[1];
            assert_eq!(observation.current, 0);
            assert_eq!(observation.acquired, observation.released);
        }
    }
    #[test]
    fn failed_construction_keeps_the_charge_through_payload_cleanup() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let drops = Arc::new(AtomicUsize::new(0));
        let failure = catch_unwind(AssertUnwindSafe(|| {
            let charge =
                context(&account).hold(Layout::new::<Allocation<Payload>>().size() as u64 + 1);
            let allocation = Box::new(Allocation::new(Payload {
                account: account.clone(),
                drops: drops.clone(),
                panic: false,
            }));
            Strong::finish(allocation, Some(charge.into()))
        }));
        assert!(failure.is_err());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().memory[1].current, 0);
    }
    #[test]
    fn array_to_slice_coercion_preserves_length_and_real_empty_allocation_identity() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let charge = Allocation::<[u64; 3]>::charge(Some(&context(&account)));
        let allocation: Box<Allocation<[u64]>> = Box::new(Allocation::new([3, 5, 7]));
        let value = Strong::finish(allocation, charge);
        assert_eq!(&*value.clone(), &[3, 5, 7]);
        let empty = || {
            let charge = Allocation::<[u64; 0]>::charge(Some(&context(&account)));
            let allocation: Box<Allocation<[u64]>> = Box::new(Allocation::new([]));
            Strong::finish(allocation, charge)
        };
        let left = empty();
        let right = empty();
        assert!(left.is_empty());
        assert!(left.same_allocation(&left.clone()));
        assert!(!left.same_allocation(&right));
        drop((value, left, right));
        assert_eq!(account.snapshot().memory[1].current, 0);
    }
    #[test]
    fn zst_and_borrowed_payloads_keep_distinct_owners_and_one_last_drop() {
        let left = Strong::new_in((), None);
        let right = Strong::new_in((), None);
        assert!(!left.same_allocation(&right));
        struct Borrowed<'a>(&'a Cell<u64>);
        impl Drop for Borrowed<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let drops = Cell::new(0);
        let value = Strong::new_in(Borrowed(&drops), None);
        let alias = value.clone();
        drop(value);
        assert_eq!(drops.get(), 0);
        drop(alias);
        assert_eq!(drops.get(), 1);
    }
    #[test]
    fn overflow_rejection_precedes_any_reference_count_mutation() {
        for initial in [0, isize::MAX as usize, usize::MAX] {
            let strong = AtomicUsize::new(initial);
            assert!(catch_unwind(|| increment(&strong)).is_err());
            assert_eq!(strong.load(Ordering::Relaxed), initial);
        }
    }
    #[test]
    fn concurrent_aliases_release_one_charge_and_publish_to_the_worker_last_owner() {
        let engine = EngineAccounting::default();
        let account = engine.query();
        let drops = Arc::new(AtomicUsize::new(0));
        let value = erased(&account, &drops, false);
        std::thread::scope(|scope| {
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let alias = value.clone();
            let child_barrier = barrier.clone();
            scope.spawn(move || {
                child_barrier.wait();
                for _ in 0..16 {
                    assert_eq!(alias.clone().read(), 19);
                }
                drop(alias);
            });
            drop(value);
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            barrier.wait();
        });
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert_eq!(account.snapshot().memory[1].current, 0);
    }

    #[derive(Debug)]
    struct AncestorPayload {
        engine: EngineAccounting,
        drops: Arc<AtomicUsize>,
        panic: bool,
    }
    impl Drop for AncestorPayload {
        fn drop(&mut self) {
            assert!(
                self.engine.snapshot().memory[MemoryKind::Accounting as usize].current
                    >= EngineAccounting::allocation_bytes() + Strong::<Self>::allocation_bytes()
            );
            self.drops.fetch_add(1, Ordering::Relaxed);
            assert!(!self.panic, "synthetic ancestor payload drop");
        }
    }
    #[test]
    fn ancestor_charges_survive_construction_failure_and_panicking_last_drop() {
        for (construction_failure, payload_panic) in [(false, false), (false, true), (true, false)]
        {
            let engine = EngineAccounting::default();
            let drops = Arc::new(AtomicUsize::new(0));
            let baseline = engine.snapshot();
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let payload = AncestorPayload {
                    engine: engine.clone(),
                    drops: drops.clone(),
                    panic: payload_panic,
                };
                let value = if construction_failure {
                    let charge = SharedCharge::Engine(
                        engine.hold_accounting(Strong::<AncestorPayload>::allocation_bytes() + 1),
                    );
                    Strong::finish(Box::new(Allocation::new(payload)), Some(charge))
                } else {
                    Strong::new_for_engine(payload, &engine)
                };
                let alias = value.clone();
                assert!(value.engine_owned());
                drop(value);
                assert_eq!(drops.load(Ordering::Relaxed), 0);
                drop(alias);
            }));
            assert_eq!(outcome.is_err(), construction_failure || payload_panic);
            assert_eq!(drops.load(Ordering::Relaxed), 1);
            assert_eq!(
                engine.snapshot().memory[MemoryKind::Accounting as usize].current,
                baseline.memory[MemoryKind::Accounting as usize].current
            );
            engine.snapshot().assert_memory_released();
        }
    }
}
