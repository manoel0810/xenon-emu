// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xobject.h / xobject.cc     X_DISPATCH_HEADER, X_OBJECT_HEADER, X_OBJECT_TYPES, kHandleBase,
//                                               CreateNative, StashHandle (kXObjSignature)
//   src/xenia/kernel/util/object_table.cc/.h    AddHandle, FindFreeSlot, TranslateHandle, Retain/Release/Remove
//   src/xenia/kernel/xevent.cc, xsemaphore.cc   native KEVENT / KSEMAPHORE initialization and signal state
//   src/xenia/kernel/xthread.h / xthread.cc     X_KTHREAD layout, XThread::InitializeGuestObject
//
//! Kernel objects and the handle table. As in Xenia, every object reachable by handle lives in one table (handles
//! 0xF8000000 + 4 * slot; sockets without the base), and dispatcher objects keep their state in a guest
//! X_DISPATCH_HEADER: `type` at +0 and `signal_state` at +4. Objects created through Nt* calls get a native object
//! (X_OBJECT_HEADER + body) in guest memory with their handle stashed in the header's wait list, like Xenia's
//! CreateNative/StashHandle; objects the guest initializes itself (KeInitializeEvent, ...) are operated on in place.

use std::collections::HashMap;

use super::guest_mem::GuestMem;

pub const HANDLE_BASE: u32 = 0xF800_0000;
/// NtCurrentThread() pseudo handle.
pub const CURRENT_THREAD_HANDLE: u32 = 0xFFFF_FFFE;
/// 'XEN\0' in X_DISPATCH_HEADER.wait_list.flink marks a header whose blink holds our handle.
pub const XOBJ_SIGNATURE: u32 = 0x5845_4E00;

pub const STATUS_SUCCESS: u64 = 0;
pub const STATUS_INVALID_HANDLE: u64 = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: u64 = 0xC000_000D;
pub const STATUS_OBJECT_TYPE_MISMATCH: u64 = 0xC000_0024;
pub const STATUS_SEMAPHORE_LIMIT_EXCEEDED: u64 = 0xC000_0047;
pub const STATUS_UNSUCCESSFUL: u64 = 0xC000_0001;

// X_DISPATCH_HEADER
pub const HEADER_TYPE: u64 = 0x00;
pub const HEADER_SIGNAL_STATE: u64 = 0x04;
pub const HEADER_WAIT_LIST: u64 = 0x08;
/// X_OBJECT_HEADER, which precedes a native object.
pub const OBJECT_HEADER_SIZE: u32 = 0x10;

// X_OBJECT_TYPES
pub const EVENT_NOTIFICATION_OBJECT: u8 = 0x0;
pub const EVENT_SYNCHRONIZATION_OBJECT: u8 = 0x1;
pub const MUTANT_OBJECT: u8 = 0x2;
pub const SEMAPHORE_OBJECT: u8 = 0x5;
pub const THREAD_OBJECT: u8 = 0x6;
pub const TIMER_NOTIFICATION_OBJECT: u8 = 0x8;
pub const TIMER_SYNCHRONIZATION_OBJECT: u8 = 0x9;

pub const KEVENT_SIZE: u32 = 0x10;
pub const KSEMAPHORE_SIZE: u32 = 0x14;
pub const KSEMAPHORE_LIMIT: u64 = 0x10;
pub const KTHREAD_SIZE: u32 = 0xAB0;

/// XObject::Type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Event,
    Semaphore,
    Mutant,
    Timer,
    Thread,
    IoCompletion,
    File,
    Socket,
    NotifyListener,
    Enumerator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelObject {
    pub kind: ObjectKind,
    /// Guest object (dispatcher header for dispatcher objects, X_KTHREAD for threads); 0 if it has none.
    pub guest_ptr: u32,
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    object: u32,
    refs: u32,
}

#[derive(Default)]
pub struct ObjectTable {
    slots: Vec<Option<Slot>>,
    last_free: usize,
    objects: HashMap<u32, KernelObject>,
    next_object: u32,
    by_guest_ptr: HashMap<u32, u32>,
}

impl ObjectTable {
    fn handle_for(slot: usize, kind: ObjectKind) -> u32 {
        let h = (slot as u32) << 2;
        if kind == ObjectKind::Socket { h } else { h + HANDLE_BASE }
    }

    fn slot_of(handle: u32) -> usize {
        ((handle & !HANDLE_BASE) >> 2) as usize
    }

    /// FindFreeSlot: the first free slot scanning from `last_free` (which only moves when the table grows, to the
    /// first new slot), growing the table when full.
    fn free_slot(&mut self) -> usize {
        let n = self.slots.len();
        for i in 0..n {
            let s = (self.last_free + i) % n;
            if self.slots[s].is_none() {
                return s;
            }
        }
        self.slots.resize(n.max(16) * 2, None);
        self.last_free = n;
        n
    }

    /// Registers a new object and returns its first handle.
    pub fn create(&mut self, object: KernelObject) -> u32 {
        let id = self.next_object;
        self.next_object += 1;
        if object.guest_ptr != 0 {
            self.by_guest_ptr.insert(object.guest_ptr, id);
        }
        let kind = object.kind;
        self.objects.insert(id, object);
        self.add_handle(id, kind)
    }

    fn add_handle(&mut self, id: u32, kind: ObjectKind) -> u32 {
        let slot = self.free_slot();
        self.slots[slot] = Some(Slot { object: id, refs: 1 });
        Self::handle_for(slot, kind)
    }

    /// TranslateHandle: NtCurrentThread() (-2) is `current_thread`, -1 is no handle.
    pub fn translate(handle: u32, current_thread: u32) -> u32 {
        match handle {
            CURRENT_THREAD_HANDLE => current_thread,
            0xFFFF_FFFF => 0,
            h => h,
        }
    }

    fn slot(&self, handle: u32) -> Option<Slot> {
        if handle == 0 {
            return None;
        }
        self.slots.get(Self::slot_of(handle)).copied().flatten()
    }

    pub fn lookup(&self, handle: u32, current_thread: u32) -> Option<&KernelObject> {
        let s = self.slot(Self::translate(handle, current_thread))?;
        self.objects.get(&s.object)
    }

    pub fn lookup_mut(&mut self, handle: u32, current_thread: u32) -> Option<&mut KernelObject> {
        let s = self.slot(Self::translate(handle, current_thread))?;
        self.objects.get_mut(&s.object)
    }

    /// The handle of the object whose guest object is `guest_ptr`, if any.
    pub fn handle_of_guest_ptr(&self, guest_ptr: u32) -> Option<u32> {
        let id = *self.by_guest_ptr.get(&guest_ptr)?;
        let kind = self.objects.get(&id)?.kind;
        self.slots.iter().position(|s| matches!(s, Some(Slot { object, .. }) if *object == id)).map(|slot| Self::handle_for(slot, kind))
    }

    /// RetainHandle.
    pub fn retain(&mut self, handle: u32, current_thread: u32) -> u64 {
        let slot = Self::slot_of(Self::translate(handle, current_thread));
        match self.slots.get_mut(slot) {
            Some(Some(s)) => {
                s.refs += 1;
                STATUS_SUCCESS
            }
            _ => STATUS_INVALID_HANDLE,
        }
    }

    /// ReleaseHandle (NtClose): drops one reference; the handle goes away with the last one. Returns the status and
    /// the object if this was its last handle.
    pub fn release(&mut self, handle: u32, current_thread: u32) -> (u64, Option<KernelObject>) {
        let handle = Self::translate(handle, current_thread);
        if handle == 0 {
            return (STATUS_INVALID_HANDLE, None);
        }
        let slot = Self::slot_of(handle);
        let Some(Some(s)) = self.slots.get_mut(slot) else { return (STATUS_INVALID_HANDLE, None) };
        s.refs -= 1;
        if s.refs > 0 {
            return (STATUS_SUCCESS, None);
        }
        let id = s.object;
        self.slots[slot] = None;
        let still_referenced = self.slots.iter().any(|e| matches!(e, Some(Slot { object, .. }) if *object == id));
        if still_referenced {
            return (STATUS_SUCCESS, None);
        }
        let object = self.objects.remove(&id);
        if let Some(o) = &object {
            self.by_guest_ptr.remove(&o.guest_ptr);
        }
        (STATUS_SUCCESS, object)
    }

    /// DuplicateHandle: a new handle to the same object.
    pub fn duplicate(&mut self, handle: u32, current_thread: u32) -> Result<u32, u64> {
        let s = self.slot(Self::translate(handle, current_thread)).ok_or(STATUS_INVALID_HANDLE)?;
        let kind = self.objects[&s.object].kind;
        Ok(self.add_handle(s.object, kind))
    }
}

/// Writes a dispatcher header: type, size (in dwords) and signal state; the wait list is left to the caller.
pub fn init_header(mem: &mut dyn GuestMem, ptr: u64, object_type: u8, size: u32, signal_state: u32) {
    mem.write_u8(ptr + HEADER_TYPE, object_type);
    mem.write_u8(ptr + 1, 0);
    mem.write_u8(ptr + 2, (size / 4) as u8);
    mem.write_u8(ptr + 3, 0);
    mem.write_u32(ptr + HEADER_SIGNAL_STATE, signal_state);
}

/// StashHandle: marks a native object as ours by storing the handle in its wait list.
pub fn stash_handle(mem: &mut dyn GuestMem, ptr: u64, handle: u32) {
    mem.write_u32(ptr + HEADER_WAIT_LIST, XOBJ_SIGNATURE);
    mem.write_u32(ptr + HEADER_WAIT_LIST + 4, handle);
}

/// KeSetEvent / NtSetEvent (XEvent::Set): signaled; returns 1 as Xenia does.
pub fn set_event(mem: &mut dyn GuestMem, event: u64) -> u64 {
    mem.write_u32(event + HEADER_SIGNAL_STATE, 1);
    1
}

/// KeResetEvent / NtClearEvent (XEvent::Reset): not signaled; returns 1 as Xenia does.
pub fn reset_event(mem: &mut dyn GuestMem, event: u64) -> u64 {
    mem.write_u32(event + HEADER_SIGNAL_STATE, 0);
    1
}

/// KePulseEvent / NtPulseEvent (XEvent::Pulse): releases waiters (none can wait yet) and leaves the event reset;
/// returns the previous signal state.
pub fn pulse_event(mem: &mut dyn GuestMem, event: u64) -> u64 {
    let previous = mem.read_u32(event + HEADER_SIGNAL_STATE);
    mem.write_u32(event + HEADER_SIGNAL_STATE, 0);
    previous as u64
}

/// XSemaphore::ReleaseSemaphore on a KSEMAPHORE: count += release unless it would pass the limit.
/// Returns (success, previous count).
pub fn release_semaphore(mem: &mut dyn GuestMem, semaphore: u64, release: u32) -> (bool, u32) {
    let previous = mem.read_u32(semaphore + HEADER_SIGNAL_STATE);
    let limit = mem.read_u32(semaphore + KSEMAPHORE_LIMIT) as i64;
    if release as i32 <= 0 || previous as i32 as i64 + release as i32 as i64 > limit {
        return (false, previous);
    }
    mem.write_u32(semaphore + HEADER_SIGNAL_STATE, previous.wrapping_add(release));
    (true, previous)
}

/// Parameters of XThread::InitializeGuestObject that do not depend on the process block.
pub struct KThreadInit {
    pub thread_id: u32,
    pub start_address: u32,
    pub stack_base: u32,
    pub stack_limit: u32,
    pub creation_flags: u32,
}

/// X_KTHREAD (0xAB0 bytes) for a new thread, as XThread::InitializeGuestObject fills it, minus the fields taken from
/// the KPROCESS (priorities, quantum, process type) that is not emulated yet and the KPRCB pointers (set when the
/// thread gets its KPCR, see `set_kthread_prcb`).
pub fn init_kthread(mem: &mut dyn GuestMem, kthread: u64, p: &KThreadInit) {
    for off in (0..KTHREAD_SIZE as u64).step_by(4) {
        mem.write_u32(kthread + off, 0);
    }
    let k = kthread as u32;
    mem.write_u8(kthread + HEADER_TYPE, THREAD_OBJECT);
    mem.write_u8(kthread + 0xBC, (p.creation_flags & 1 != 0) as u8); // suspend_count (X_CREATE_SUSPENDED)
    mem.write_u32(kthread + 0x10, k + 0x10); // mutants_list
    mem.write_u32(kthread + 0x14, k + 0x10);
    mem.write_u32(kthread + 0x5C, p.stack_base);
    mem.write_u32(kthread + 0x60, p.stack_limit);
    mem.write_u32(kthread + 0x64, p.stack_base.wrapping_sub(240)); // stack_kernel
    mem.write_u8(kthread + 0x8B, 1); // may_queue_apcs
    mem.write_u32(kthread + 0x9C, 0xFDFF_D7FF); // msr_mask
    mem.write_u32(kthread + 0xD0, p.stack_base); // stack_alloc_base
    mem.write_u32(kthread + 0x144, k + 0x144); // timer_list
    mem.write_u32(kthread + 0x148, k + 0x144);
    mem.write_u32(kthread + 0x14C, p.thread_id);
    mem.write_u32(kthread + 0x150, p.start_address);
    mem.write_u32(kthread + 0x154, k + 0x154); // unk_154
    mem.write_u32(kthread + 0x158, k + 0x154);
}

/// Stack fields of a KTHREAD (stack_base, stack_limit, stack_kernel, stack_alloc_base), known once the thread's stack
/// is placed.
pub fn set_kthread_stack(mem: &mut dyn GuestMem, kthread: u64, stack_base: u32, stack_limit: u32) {
    mem.write_u32(kthread + 0x5C, stack_base);
    mem.write_u32(kthread + 0x60, stack_limit);
    mem.write_u32(kthread + 0x64, stack_base.wrapping_sub(240));
    mem.write_u32(kthread + 0xD0, stack_base);
}

/// a_prcb_ptr / another_prcb_ptr of a KTHREAD: the KPRCB inside the thread's KPCR.
pub fn set_kthread_prcb(mem: &mut dyn GuestMem, kthread: u64, kprcb: u32) {
    mem.write_u32(kthread + 0xC0, kprcb);
    mem.write_u32(kthread + 0xC4, kprcb);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;

    fn ev(ptr: u32) -> KernelObject {
        KernelObject { kind: ObjectKind::Event, guest_ptr: ptr }
    }

    #[test]
    fn handles_follow_xenia_numbering_and_reuse() {
        let mut t = ObjectTable::default();
        let a = t.create(ev(0x100));
        let b = t.create(ev(0x200));
        assert_eq!((a, b), (0xF800_0000, 0xF800_0004));
        let sock = t.create(KernelObject { kind: ObjectKind::Socket, guest_ptr: 0 });
        assert_eq!(sock, 8, "sockets have no handle base");
        assert_eq!(t.lookup(b, 0).map(|o| o.guest_ptr), Some(0x200));
        assert_eq!(t.handle_of_guest_ptr(0x200), Some(b));
        // close frees the slot; the next object reuses it
        let (status, gone) = t.release(a, 0);
        assert_eq!((status, gone.map(|o| o.guest_ptr)), (STATUS_SUCCESS, Some(0x100)));
        assert!(t.lookup(a, 0).is_none());
        assert_eq!(t.release(a, 0).0, STATUS_INVALID_HANDLE);
        assert_eq!(t.create(ev(0x300)), 0xF800_0000);
    }

    #[test]
    fn retain_duplicate_and_current_thread() {
        let mut t = ObjectTable::default();
        let thread = t.create(KernelObject { kind: ObjectKind::Thread, guest_ptr: 0x7000 });
        assert_eq!(t.lookup(CURRENT_THREAD_HANDLE, thread).map(|o| o.guest_ptr), Some(0x7000));
        assert!(t.lookup(CURRENT_THREAD_HANDLE, 0).is_none());
        let dup = t.duplicate(CURRENT_THREAD_HANDLE, thread).unwrap();
        assert_ne!(dup, thread);
        assert_eq!(t.release(thread, 0), (STATUS_SUCCESS, None), "the duplicate keeps the object alive");
        assert_eq!(t.lookup(dup, 0).map(|o| o.kind), Some(ObjectKind::Thread));
        assert_eq!(t.retain(dup, 0), STATUS_SUCCESS);
        assert_eq!(t.release(dup, 0), (STATUS_SUCCESS, None));
        assert!(t.release(dup, 0).1.is_some());
        assert_eq!(t.duplicate(0xF800_0100, 0), Err(STATUS_INVALID_HANDLE));
    }

    #[test]
    fn event_and_semaphore_state_lives_in_the_guest_header() {
        let mut m = FakeMem::default();
        init_header(&mut m, 0x1000, EVENT_SYNCHRONIZATION_OBJECT, KEVENT_SIZE, 0);
        assert_eq!((m.read_u8(0x1000), m.read_u8(0x1002), m.read_u32(0x1004)), (1, 4, 0));
        assert_eq!(set_event(&mut m, 0x1000), 1);
        assert_eq!(m.read_u32(0x1004), 1);
        assert_eq!(pulse_event(&mut m, 0x1000), 1);
        assert_eq!(m.read_u32(0x1004), 0);
        stash_handle(&mut m, 0x1000, 0xF800_0004);
        assert_eq!((m.read_u32(0x1008), m.read_u32(0x100C)), (XOBJ_SIGNATURE, 0xF800_0004));
        // semaphore count 1, limit 2
        init_header(&mut m, 0x2000, SEMAPHORE_OBJECT, KSEMAPHORE_SIZE, 1);
        m.write_u32(0x2010, 2);
        assert_eq!(release_semaphore(&mut m, 0x2000, 1), (true, 1));
        assert_eq!(release_semaphore(&mut m, 0x2000, 1), (false, 2), "over the limit");
        assert_eq!(m.read_u32(0x2004), 2);
    }

    #[test]
    fn kthread_fields() {
        let mut m = FakeMem::default();
        init_kthread(&mut m, 0x4000_0000, &KThreadInit { thread_id: 7, start_address: 0x9226_1CE8, stack_base: 0x7200_0000, stack_limit: 0x71FC_0000, creation_flags: 1 });
        assert_eq!(m.read_u8(0x4000_0000), THREAD_OBJECT);
        assert_eq!((m.read_u32(0x4000_005C), m.read_u32(0x4000_0060), m.read_u32(0x4000_0064)), (0x7200_0000, 0x71FC_0000, 0x7200_0000 - 240));
        assert_eq!((m.read_u32(0x4000_014C), m.read_u32(0x4000_0150), m.read_u8(0x4000_00BC)), (7, 0x9226_1CE8, 1));
        assert_eq!(m.read_u32(0x4000_0144), 0x4000_0144);
    }

    mod exports {
        use super::super::*;
        use crate::kernel::KernelDispatcher;
        use crate::memory::MemoryBus;

        fn call(k: &mut KernelDispatcher, bus: &mut MemoryBus, ordinal: u16, args: &[u64]) -> u64 {
            k.dispatch("xboxkrnl.exe", ordinal, args, bus)
        }

        #[test]
        fn nt_events_and_semaphores_are_native_objects() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            // NtCreateEvent(handle_ptr, obj_attributes, event_type = synchronization, initial_state = 1)
            assert_eq!(call(&mut k, &mut bus, 0x00D1, &[0x4100_0000, 0, 1, 1]), 0);
            let h = bus.read_u32(0x4100_0000).unwrap();
            assert_eq!(h & 0xF800_0003, HANDLE_BASE);
            let ev = k.objects.lookup(h, 0).unwrap().guest_ptr as u64;
            assert_eq!((bus.read_u8(ev).unwrap(), bus.read_u32(ev + 4).unwrap()), (EVENT_SYNCHRONIZATION_OBJECT, 1));
            assert_eq!((bus.read_u32(ev + 8).unwrap(), bus.read_u32(ev + 12).unwrap()), (XOBJ_SIGNATURE, h));
            // NtClearEvent / NtSetEvent(previous_state_ptr) through the handle
            assert_eq!(call(&mut k, &mut bus, 0x00CE, &[h as u64]), 0);
            assert_eq!(bus.read_u32(ev + 4).unwrap(), 0);
            assert_eq!(call(&mut k, &mut bus, 0x00F6, &[h as u64, 0x4100_0010]), 0);
            assert_eq!((bus.read_u32(ev + 4).unwrap(), bus.read_u32(0x4100_0010).unwrap()), (1, 1));
            // NtCreateSemaphore(handle_ptr, obj_attributes, count 0, limit 2) and NtReleaseSemaphore
            assert_eq!(call(&mut k, &mut bus, 0x00D5, &[0x4100_0020, 0, 0, 2]), 0);
            let s = bus.read_u32(0x4100_0020).unwrap();
            assert_eq!(call(&mut k, &mut bus, 0x00F3, &[s as u64, 2, 0x4100_0024]), 0);
            assert_eq!(bus.read_u32(0x4100_0024).unwrap(), 0);
            assert_eq!(call(&mut k, &mut bus, 0x00F3, &[s as u64, 1, 0x4100_0024]), STATUS_SEMAPHORE_LIMIT_EXCEEDED);
            // wrong type, then NtClose
            assert_eq!(call(&mut k, &mut bus, 0x00F6, &[s as u64, 0]), STATUS_OBJECT_TYPE_MISMATCH);
            assert_eq!(call(&mut k, &mut bus, 0x00CF, &[h as u64]), 0);
            assert_eq!(call(&mut k, &mut bus, 0x00F6, &[h as u64, 0]), STATUS_INVALID_HANDLE);
            assert_eq!(call(&mut k, &mut bus, 0x00CF, &[h as u64]), STATUS_INVALID_HANDLE);
        }

        #[test]
        fn current_thread_object_by_handle_and_id() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            let init = KThreadInit { thread_id: 1, start_address: 0x9219_6610, stack_base: 0x7140_0000, stack_limit: 0x7130_0000, creation_flags: 0 };
            let (h, kthread) = k.create_thread_object(&mut bus, &init);
            k.current_thread = h;
            // ObReferenceObjectByHandle(NtCurrentThread(), ExThreadObjectType import record, out) as the dash calls it;
            // the 4th register (a code address in the dash) must not be written
            bus.write_u32(0x4100_0104, 0xEEEE_EEEE).unwrap();
            assert_eq!(call(&mut k, &mut bus, 0x0110, &[0xFFFF_FFFE, 0x0001_001B, 0x4100_0100, 0x4100_0104]), 0);
            assert_eq!((bus.read_u32(0x4100_0100).unwrap(), bus.read_u32(0x4100_0104).unwrap()), (kthread, 0xEEEE_EEEE));
            assert_eq!(bus.read_u8(kthread as u64).unwrap(), THREAD_OBJECT);
            // ObDereferenceObject balances it; NtDuplicateObject(-2) gives a real handle
            assert_eq!(call(&mut k, &mut bus, 0x0105, &[kthread as u64]), 0);
            assert_eq!(call(&mut k, &mut bus, 0x00DA, &[0xFFFF_FFFE, 0x4100_0108, 0]), 0);
            let dup = bus.read_u32(0x4100_0108).unwrap();
            assert!(dup != h && k.objects.lookup(dup, 0).map(|o| o.guest_ptr) == Some(kthread));
            // ObLookupThreadByThreadId
            assert_eq!(call(&mut k, &mut bus, 0x010B, &[1, 0x4100_010C]), 0);
            assert_eq!(bus.read_u32(0x4100_010C).unwrap(), kthread);
            assert_eq!(call(&mut k, &mut bus, 0x010B, &[99, 0x4100_010C]), STATUS_INVALID_PARAMETER);
            // without a current thread, -2 is not a handle
            k.current_thread = 0;
            assert_eq!(call(&mut k, &mut bus, 0x0110, &[0xFFFF_FFFE, 0, 0x4100_0100]), STATUS_INVALID_HANDLE);
        }

        #[test]
        fn ke_objects_are_guest_headers() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            // KeInitializeSemaphore(sem, count 1, limit 3); KeReleaseSemaphore(sem, increment, adjustment 1, wait)
            call(&mut k, &mut bus, 0x0074, &[0x4100_0200, 1, 3]);
            assert_eq!((bus.read_u8(0x4100_0200).unwrap(), bus.read_u32(0x4100_0204).unwrap(), bus.read_u32(0x4100_0210).unwrap()), (SEMAPHORE_OBJECT, 1, 3));
            assert_eq!(call(&mut k, &mut bus, 0x0088, &[0x4100_0200, 0, 1, 0]), 1);
            assert_eq!(bus.read_u32(0x4100_0204).unwrap(), 2);
            // KeInitializeEvent(notification, not signaled); KeSetEvent returns 1 like Xenia
            call(&mut k, &mut bus, 0x0070, &[0x4100_0300, 0, 0]);
            assert_eq!(call(&mut k, &mut bus, 0x009D, &[0x4100_0300, 0, 0]), 1);
            assert_eq!(bus.read_u32(0x4100_0304).unwrap(), 1);
            assert_eq!(call(&mut k, &mut bus, 0x007F, &[0x4100_0300, 0, 0]), 1, "KePulseEvent returns the previous state");
            assert_eq!(bus.read_u32(0x4100_0304).unwrap(), 0);
        }
    }
}
