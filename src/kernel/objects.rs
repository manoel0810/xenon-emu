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
pub const STATUS_MUTANT_NOT_OWNED: u64 = 0xC000_0046;

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
/// X_KMUTANT: header, list entry, owner (X_KTHREAD) at +0x18, abandoned at +0x1C.
pub const KMUTANT_SIZE: u32 = 0x20;
pub const KMUTANT_OWNER: u64 = 0x18;
/// X_KTIMER: header, due time (+0x10), list entry, dpc (+0x20), period in ms (+0x24).
pub const KTIMER_SIZE: u32 = 0x28;
pub const KTIMER_DUE_TIME: u64 = 0x10;
pub const KTIMER_DPC: u64 = 0x20;
pub const KTIMER_PERIOD: u64 = 0x24;
/// X_DISPATCH_HEADER.inserted: the timer is in the timer list (set and not yet expired).
pub const HEADER_INSERTED: u64 = 0x03;
/// X_RTL_CRITICAL_SECTION: a synchronization-event header, then lock_count, recursion_count, owning_thread.
pub const CS_LOCK_COUNT: u64 = 0x10;
pub const CS_RECURSION_COUNT: u64 = 0x14;
pub const CS_OWNING_THREAD: u64 = 0x18;

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

/// A KMUTANT: free when signal_state > 0; the owner may acquire it again (recursion makes signal_state negative).
pub fn init_mutant(mem: &mut dyn GuestMem, mutant: u64, owner: u32) {
    init_header(mem, mutant, MUTANT_OBJECT, KMUTANT_SIZE, if owner != 0 { 0 } else { 1 });
    mem.write_u32(mutant + KMUTANT_OWNER, owner);
}

/// Releases a mutant held by `thread` (XMutant::ReleaseMutant): Ok(previous signal state), or not owned.
pub fn release_mutant(mem: &mut dyn GuestMem, mutant: u64, thread: u32) -> Result<u32, u64> {
    if mem.read_u32(mutant + KMUTANT_OWNER) != thread {
        return Err(STATUS_MUTANT_NOT_OWNED);
    }
    let previous = mem.read_u32(mutant + HEADER_SIGNAL_STATE);
    let state = previous.wrapping_add(1);
    mem.write_u32(mutant + HEADER_SIGNAL_STATE, state);
    if state as i32 > 0 {
        mem.write_u32(mutant + KMUTANT_OWNER, 0);
    }
    Ok(previous)
}

/// RtlInitializeCriticalSection(AndSpinCount): a synchronization event with lock_count -1 and no owner.
pub fn init_critical_section(mem: &mut dyn GuestMem, cs: u64, spin_count_div_256: u8) {
    init_header(mem, cs, EVENT_SYNCHRONIZATION_OBJECT, 0x10, 0);
    mem.write_u8(cs + 1, spin_count_div_256);
    mem.write_u32(cs + CS_LOCK_COUNT, u32::MAX);
    mem.write_u32(cs + CS_RECURSION_COUNT, 0);
    mem.write_u32(cs + CS_OWNING_THREAD, 0);
}

/// RtlEnterCriticalSection for `thread`: true if the section is now held; false if the thread has to wait on the
/// section's event (lock_count already counts it as a waiter).
pub fn enter_critical_section(mem: &mut dyn GuestMem, cs: u64, thread: u32) -> bool {
    let lock_count = mem.read_u32(cs + CS_LOCK_COUNT) as i32;
    if thread != 0 && mem.read_u32(cs + CS_OWNING_THREAD) == thread {
        mem.write_u32(cs + CS_LOCK_COUNT, lock_count.wrapping_add(1) as u32);
        let recursion = mem.read_u32(cs + CS_RECURSION_COUNT);
        mem.write_u32(cs + CS_RECURSION_COUNT, recursion.wrapping_add(1));
        return true;
    }
    mem.write_u32(cs + CS_LOCK_COUNT, lock_count.wrapping_add(1) as u32);
    if lock_count == -1 {
        take_critical_section(mem, cs, thread);
        return true;
    }
    false
}

/// The thread becomes the owner (after finding the section free, or after its wait on the section's event).
pub fn take_critical_section(mem: &mut dyn GuestMem, cs: u64, thread: u32) {
    mem.write_u32(cs + CS_OWNING_THREAD, thread);
    mem.write_u32(cs + CS_RECURSION_COUNT, 1);
}

/// RtlLeaveCriticalSection: drops one level; on the last one releases the section and, if threads wait, signals its
/// event so that one of them takes it.
pub fn leave_critical_section(mem: &mut dyn GuestMem, cs: u64) {
    let recursion = mem.read_u32(cs + CS_RECURSION_COUNT).wrapping_sub(1);
    mem.write_u32(cs + CS_RECURSION_COUNT, recursion);
    let lock_count = (mem.read_u32(cs + CS_LOCK_COUNT) as i32).wrapping_sub(1);
    mem.write_u32(cs + CS_LOCK_COUNT, lock_count as u32);
    if recursion != 0 {
        return;
    }
    mem.write_u32(cs + CS_OWNING_THREAD, 0);
    if lock_count != -1 {
        set_event(mem, cs);
    }
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

/// X_KTHREAD.suspend_count (u8), terminated (u8) and exit_status.
pub const KTHREAD_SUSPEND_COUNT: u64 = 0xBC;
pub const KTHREAD_TERMINATED: u64 = 0xBE;
pub const KTHREAD_EXIT_STATUS: u64 = 0x140;

/// XThread::Exit: the thread is terminated and its header signaled (waiters on the thread wake) with the exit code.
pub fn exit_kthread(mem: &mut dyn GuestMem, kthread: u64, exit_code: u32) {
    mem.write_u8(kthread + KTHREAD_TERMINATED, 1);
    mem.write_u32(kthread + HEADER_SIGNAL_STATE, 1);
    mem.write_u32(kthread + KTHREAD_EXIT_STATUS, exit_code);
}

/// XThread::Suspend / Resume on the KTHREAD's suspend count; returns the previous count. A thread runs only while
/// the count is 0.
pub fn adjust_suspend_count(mem: &mut dyn GuestMem, kthread: u64, delta: i8) -> u32 {
    let previous = mem.read_u8(kthread + KTHREAD_SUSPEND_COUNT);
    mem.write_u8(kthread + KTHREAD_SUSPEND_COUNT, previous.wrapping_add(delta as u8));
    previous as u32
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

        use crate::kernel::scheduler::{evaluate, WaitOutcome, WakeAction, STATUS_TIMEOUT};

        /// A kernel whose calling thread has the given KTHREAD.
        fn as_thread(k: &mut KernelDispatcher, kthread: u32) {
            k.current_kthread = kthread;
            k.pending_wait = None;
        }

        #[test]
        fn ke_waits_complete_at_once_or_park_the_thread() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            as_thread(&mut k, 0x4200_0000);
            // a signaled synchronization event satisfies the wait at once and resets
            call(&mut k, &mut bus, 0x0070, &[0x4100_0000, 1, 1]);
            assert_eq!(call(&mut k, &mut bus, 0x00B0, &[0x4100_0000, 0, 0, 0, 0]), 0);
            assert_eq!((bus.read_u32(0x4100_0004).unwrap(), k.pending_wait.is_none()), (0, true));
            // not signaled, timeout 0: only polls
            bus.write_u64(0x4100_0100, 0).unwrap();
            assert_eq!(call(&mut k, &mut bus, 0x00B0, &[0x4100_0000, 0, 0, 0, 0x4100_0100]), STATUS_TIMEOUT);
            assert!(k.pending_wait.is_none());
            // not signaled, no timeout: the thread is parked until KeSetEvent
            call(&mut k, &mut bus, 0x00B0, &[0x4100_0000, 0, 0, 0, 0]);
            let wait = k.pending_wait.take().expect("parked");
            assert_eq!((wait.objects.clone(), wait.deadline), (vec![0x4100_0000], None));
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Pending);
            call(&mut k, &mut bus, 0x009D, &[0x4100_0000, 0, 0]);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Done(0));
            // KeWaitForMultipleObjects WaitAny (1) returns the index of the signaled object
            call(&mut k, &mut bus, 0x0070, &[0x4100_0010, 0, 0]);
            call(&mut k, &mut bus, 0x0070, &[0x4100_0020, 0, 1]);
            bus.write_u32(0x4100_0200, 0x4100_0010).unwrap();
            bus.write_u32(0x4100_0204, 0x4100_0020).unwrap();
            assert_eq!(call(&mut k, &mut bus, 0x00AF, &[2, 0x4100_0200, 1, 0, 0, 0, 0, 0]), 1);
            // WaitAll (0) parks while one is not signaled; a relative timeout gives a deadline
            bus.write_u64(0x4100_0108, (-10_000i64) as u64).unwrap();
            call(&mut k, &mut bus, 0x00AF, &[2, 0x4100_0200, 0, 0, 0, 0, 0x4100_0108, 0]);
            let all = k.pending_wait.take().expect("parked");
            assert!(all.wait_all && all.deadline == Some(bus.timebase.now() + 50_000));
        }

        #[test]
        fn nt_waits_use_handles() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            as_thread(&mut k, 0x4200_0000);
            call(&mut k, &mut bus, 0x00D1, &[0x4100_0000, 0, 0, 0]);
            let ev = bus.read_u32(0x4100_0000).unwrap() as u64;
            assert_eq!(call(&mut k, &mut bus, 0x00FD, &[ev, 0, 0, 0]), 0);
            assert!(k.pending_wait.is_some(), "a not-signaled event parks the caller");
            k.pending_wait = None;
            assert_eq!(call(&mut k, &mut bus, 0x00FD, &[0xF800_0400, 0, 0, 0]), STATUS_INVALID_HANDLE);
            // NtWaitForMultipleObjectsEx: an object without a dispatcher header here counts as signaled (WaitAny)
            let port = k.new_handle(ObjectKind::IoCompletion);
            bus.write_u32(0x4100_0010, ev as u32).unwrap();
            bus.write_u32(0x4100_0014, port).unwrap();
            assert_eq!(call(&mut k, &mut bus, 0x00FE, &[2, 0x4100_0010, 1, 0, 0, 0]), 1);
            assert_eq!(call(&mut k, &mut bus, 0x00FE, &[0, 0x4100_0010, 1, 0, 0, 0]), STATUS_INVALID_PARAMETER);
            // NtSignalAndWaitForSingleObjectEx: signals the semaphore, then waits on the (now set) event
            call(&mut k, &mut bus, 0x00D5, &[0x4100_0020, 0, 0, 5]);
            let sem = bus.read_u32(0x4100_0020).unwrap();
            call(&mut k, &mut bus, 0x00F6, &[ev, 0]);
            assert_eq!(call(&mut k, &mut bus, 0x00FB, &[sem as u64, ev, 0, 0, 0]), 0);
            let sem_ptr = k.objects.lookup(sem, 0).unwrap().guest_ptr as u64;
            assert_eq!(bus.read_u32(sem_ptr + 4).unwrap(), 1);
        }

        #[test]
        fn mutants_have_an_owner_and_recursion() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            let (a, b) = (0x4200_0000u32, 0x4300_0000u32);
            as_thread(&mut k, a);
            // created owned by A; A acquires it again (recursion)
            call(&mut k, &mut bus, 0x00D4, &[0x4100_0000, 0, 1]);
            let m = bus.read_u32(0x4100_0000).unwrap() as u64;
            let mp = k.objects.lookup(m as u32, 0).unwrap().guest_ptr as u64;
            assert_eq!(call(&mut k, &mut bus, 0x00FD, &[m, 0, 0, 0]), 0);
            assert_eq!((bus.read_u32(mp + 4).unwrap() as i32, bus.read_u32(mp + KMUTANT_OWNER).unwrap()), (-1, a));
            // B waits; B cannot release it
            as_thread(&mut k, b);
            call(&mut k, &mut bus, 0x00FD, &[m, 0, 0, 0]);
            let wait = k.pending_wait.take().expect("B parked");
            assert_eq!(objects_release(&mut bus, mp, b), Err(STATUS_MUTANT_NOT_OWNED));
            // A releases twice; then B gets it
            as_thread(&mut k, a);
            assert_eq!(call(&mut k, &mut bus, 0x00F2, &[m, 0]), 0);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Pending, "A still holds one level");
            call(&mut k, &mut bus, 0x00F2, &[m, 0]);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Done(0));
            assert_eq!((bus.read_u32(mp + 4).unwrap(), bus.read_u32(mp + KMUTANT_OWNER).unwrap()), (0, b));
        }

        fn objects_release(bus: &mut MemoryBus, mutant: u64, thread: u32) -> Result<u32, u64> {
            super::super::release_mutant(bus, mutant, thread)
        }

        #[test]
        fn critical_sections_block_and_hand_over() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            let (a, b) = (0x4200_0000u32, 0x4300_0000u32);
            const CS: u64 = 0x4100_0000;
            call(&mut k, &mut bus, 0x012F, &[CS, 4000]);
            assert_eq!((bus.read_u8(CS + 1).unwrap(), bus.read_u32(CS + CS_LOCK_COUNT).unwrap() as i32), (16, -1));
            // A enters twice (recursion)
            as_thread(&mut k, a);
            call(&mut k, &mut bus, 0x0125, &[CS]);
            call(&mut k, &mut bus, 0x0125, &[CS]);
            assert_eq!((bus.read_u32(CS + CS_LOCK_COUNT).unwrap(), bus.read_u32(CS + CS_RECURSION_COUNT).unwrap(), bus.read_u32(CS + CS_OWNING_THREAD).unwrap()), (1, 2, a));
            // B blocks on it
            as_thread(&mut k, b);
            call(&mut k, &mut bus, 0x0125, &[CS]);
            let wait = k.pending_wait.take().expect("B parked");
            assert_eq!(wait.on_wake, WakeAction::EnterCriticalSection(CS as u32));
            assert_eq!(call(&mut k, &mut bus, 0x0141, &[CS]), 0, "TryEnter fails for B");
            // A leaves once: still owner; leaves again: the event hands the section to B
            as_thread(&mut k, a);
            call(&mut k, &mut bus, 0x0130, &[CS]);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Pending);
            call(&mut k, &mut bus, 0x0130, &[CS]);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Done(0));
            assert_eq!((bus.read_u32(CS + CS_LOCK_COUNT).unwrap(), bus.read_u32(CS + CS_RECURSION_COUNT).unwrap(), bus.read_u32(CS + CS_OWNING_THREAD).unwrap()), (0, 1, b));
            // B leaves: free again
            as_thread(&mut k, b);
            call(&mut k, &mut bus, 0x0130, &[CS]);
            assert_eq!((bus.read_u32(CS + CS_LOCK_COUNT).unwrap() as i32, bus.read_u32(CS + CS_OWNING_THREAD).unwrap()), (-1, 0));
        }

        #[test]
        fn video_variables_and_listeners() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            let vars = k.register_variable_exports(&mut bus);
            assert_eq!(vars.iter().map(|v| v.0).collect::<Vec<_>>(), vec![0x01BE, 0x01BF, 0x01C0, 0x01C1, 0x00AD]);
            assert_eq!(bus.read_u32(0x801D_0E94).unwrap(), 500);
            assert_eq!((bus.read_u32(0x801D_1210 + CS_LOCK_COUNT).unwrap() as i32, bus.read_u8(0x801D_1211).unwrap()), (-1, 40));
            // the first listener of the system area gets the startup notifications (on by default, as in Xenia)
            let first = k.dispatch("xam.xex", 0x0291, &[1, 1, 10], &mut bus) as u32;
            let first_ev = k.objects.lookup(first, 0).unwrap().guest_ptr as u64;
            assert_eq!(bus.read_u32(first_ev + 4).unwrap(), 1);
            // a later listener of the same area starts empty, its wait handle not signaled
            let h = k.dispatch("xam.xex", 0x0291, &[1, 1, 10], &mut bus) as u32;
            let ev = k.objects.lookup(h, 0).unwrap().guest_ptr as u64;
            assert_eq!(bus.read_u32(ev + 4).unwrap(), 0);
            bus.write_u32(0x4100_0000, 0xFFFF).unwrap();
            assert_eq!(k.dispatch("xam.xex", 0x028B, &[h as u64, 0, 0x4100_0000, 0x4100_0004], &mut bus), 0);
            // XNotifyBroadcast queues to it and signals the handle; XNotifyGetNext dequeues
            k.dispatch("xam.xex", 0x028E, &[0x0000_0009, 1], &mut bus);
            assert_eq!(bus.read_u32(ev + 4).unwrap(), 1);
            assert_eq!(k.dispatch("xam.xex", 0x028B, &[h as u64, 0, 0x4100_0000, 0x4100_0004], &mut bus), 1);
            assert_eq!((bus.read_u32(0x4100_0000).unwrap(), bus.read_u32(0x4100_0004).unwrap(), bus.read_u32(ev + 4).unwrap()), (9, 1, 0));
        }

        #[test]
        fn thread_exit_signals_waiters_and_suspension_follows_the_count() {
            let mut k = KernelDispatcher::new();
            let mut bus = MemoryBus::new();
            // ExCreateThread(handle_ptr, stack, tid_ptr, api, start, context, flags = CREATE_SUSPENDED)
            k.dispatch("xboxkrnl.exe", 0x000D, &[0x4100_0000, 0, 0x4100_0004, 0, 0x9226_1CE8, 0, 1], &mut bus);
            let h = bus.read_u32(0x4100_0000).unwrap() as u64;
            let kthread = k.objects.lookup(h as u32, 0).unwrap().guest_ptr as u64;
            assert_eq!(bus.read_u8(kthread + KTHREAD_SUSPEND_COUNT).unwrap(), 1);
            // NtResumeThread: previous count 1, now runnable
            assert_eq!(call(&mut k, &mut bus, 0x00F5, &[h, 0x4100_0008]), 0);
            assert_eq!((bus.read_u32(0x4100_0008).unwrap(), bus.read_u8(kthread + KTHREAD_SUSPEND_COUNT).unwrap()), (1, 0));
            assert_eq!(call(&mut k, &mut bus, 0x00FC, &[h, 0x4100_0008]), 0);
            assert_eq!(bus.read_u8(kthread + KTHREAD_SUSPEND_COUNT).unwrap(), 1);
            // another thread waits on it; ExTerminateThread (XThread::Exit) signals it with the exit code
            k.current_kthread = 0x4200_0000;
            call(&mut k, &mut bus, 0x00FD, &[h, 0, 0, 0]);
            let wait = k.pending_wait.take().expect("waiting on the thread");
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Pending);
            exit_kthread(&mut bus, kthread, 0x55);
            assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Done(0));
            assert_eq!((bus.read_u8(kthread + KTHREAD_TERMINATED).unwrap(), bus.read_u32(kthread + KTHREAD_EXIT_STATUS).unwrap()), (1, 0x55));
            assert_eq!(bus.read_u32(kthread + 4).unwrap(), 1, "a thread stays signaled once it has exited");
        }
    }
}
