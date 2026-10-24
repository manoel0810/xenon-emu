// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xthread.h                  struct XDPC (Initialize), X_KPCR / X_KPRCB
//   src/xenia/kernel/xboxkrnl/xboxkrnl_threading.cc KeInitializeDpc / KeInsertQueueDpc / KeRemoveQueueDpc
//   src/xenia/kernel/kernel_state.cc            BeginDPCImpersonation / EndDPCImpersonation
//
//! Deferred procedure calls. As in Xenia, KeInsertQueueDpc runs the DPC routine inline on the calling thread
//! (routine(dpc, context, arg1, arg2)) at DISPATCH_LEVEL with KPCR.prcb_data.dpc_active set. The kernel side
//! here only updates the XDPC and returns the call to make; the frame loop performs it on the guest thread.

use std::collections::HashSet;

use super::guest_mem::GuestMem;

// XDPC (xthread.h)
pub const XDPC_TYPE: u64 = 0x00; // be u16, 19 for a DPC
pub const XDPC_SELECTED_CPU: u64 = 0x02;
pub const XDPC_DESIRED_CPU: u64 = 0x03;
pub const XDPC_LIST_ENTRY: u64 = 0x04;
pub const XDPC_ROUTINE: u64 = 0x0C;
pub const XDPC_CONTEXT: u64 = 0x10;
pub const XDPC_ARG1: u64 = 0x14;
pub const XDPC_ARG2: u64 = 0x18;
pub const DPC_OBJECT_TYPE: u16 = 19;

// X_KPCR (xthread.h): current_irql at 0x18; prcb_data (X_KPRCB) at 0x100, dpc_active at +0x50.
pub const KPCR_CURRENT_IRQL: u64 = 0x18;
pub const KPCR_DPC_ACTIVE: u64 = 0x150;
pub const DISPATCH_LEVEL: u8 = 2;

/// A DPC routine call to run on the thread that called KeInsertQueueDpc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpcCall {
    pub dpc: u32,
    pub routine: u32,
    pub context: u32,
    pub arg1: u32,
    pub arg2: u32,
}

/// KeInitializeDpc(dpc, routine, context) -> XDPC::Initialize.
pub fn initialize_dpc(mem: &mut dyn GuestMem, dpc: u64, routine: u64, context: u64) {
    let dpc = dpc & 0xFFFF_FFFF;
    if dpc == 0 {
        return;
    }
    mem.write_u16(dpc + XDPC_TYPE, DPC_OBJECT_TYPE);
    mem.write_u8(dpc + XDPC_SELECTED_CPU, 0);
    mem.write_u8(dpc + XDPC_DESIRED_CPU, 0);
    mem.write_u32(dpc + XDPC_ROUTINE, routine as u32);
    mem.write_u32(dpc + XDPC_CONTEXT, context as u32);
}

/// KeInsertQueueDpc(dpc, arg1, arg2): stores the arguments, marks the DPC queued and returns 1 (Xenia always
/// returns 1 and dispatches inline). The routine call, if any, is returned for the caller's thread to run.
pub fn insert_queue_dpc(mem: &mut dyn GuestMem, queued: &mut HashSet<u32>, dpc: u64, arg1: u64, arg2: u64) -> (u64, Option<DpcCall>) {
    let dpc = (dpc & 0xFFFF_FFFF) as u32;
    if dpc == 0 {
        return (0, None);
    }
    let d = dpc as u64;
    mem.write_u32(d + XDPC_ARG1, arg1 as u32);
    mem.write_u32(d + XDPC_ARG2, arg2 as u32);
    queued.insert(dpc);
    let routine = mem.read_u32(d + XDPC_ROUTINE);
    let call = (routine != 0).then(|| DpcCall { dpc, routine, context: mem.read_u32(d + XDPC_CONTEXT), arg1: arg1 as u32, arg2: arg2 as u32 });
    (1, call)
}

/// KeRemoveQueueDpc(dpc): 1 if the DPC was queued (and dequeues it), else 0.
pub fn remove_queue_dpc(queued: &mut HashSet<u32>, dpc: u64) -> u64 {
    queued.remove(&((dpc & 0xFFFF_FFFF) as u32)) as u64
}

/// BeginDPCImpersonation: raise to DISPATCH_LEVEL and set dpc_active. Returns the previous IRQL, or None when the
/// thread is already inside a DPC (reentrant insert: no impersonation, like Xenia).
pub fn begin_impersonation(mem: &mut dyn GuestMem, kpcr: u64) -> Option<u8> {
    let kpcr = kpcr & 0xFFFF_FFFF;
    if mem.read_u32(kpcr + KPCR_DPC_ACTIVE) != 0 {
        return None;
    }
    let previous = mem.read_u8(kpcr + KPCR_CURRENT_IRQL);
    mem.write_u8(kpcr + KPCR_CURRENT_IRQL, DISPATCH_LEVEL);
    mem.write_u32(kpcr + KPCR_DPC_ACTIVE, 1);
    Some(previous)
}

/// EndDPCImpersonation: restore the IRQL and clear dpc_active.
pub fn end_impersonation(mem: &mut dyn GuestMem, kpcr: u64, previous_irql: u8) {
    let kpcr = kpcr & 0xFFFF_FFFF;
    mem.write_u8(kpcr + KPCR_CURRENT_IRQL, previous_irql);
    mem.write_u32(kpcr + KPCR_DPC_ACTIVE, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;

    // the dash's D3D DPC (KeInitializeDpc at frame 3 from 0x921C9DDC)
    const DPC: u64 = 0x4000_51C4;
    const ROUTINE: u64 = 0x921C_89B0;
    const CONTEXT: u64 = 0x4000_51A4;

    #[test]
    fn initialize_uses_xenia_xdpc_layout() {
        let mut m = FakeMem::default();
        for off in (0..0x1C).step_by(4) {
            m.write_u32(DPC + off, 0xEEEE_EEEE);
        }
        initialize_dpc(&mut m, DPC, ROUTINE, CONTEXT);
        assert_eq!(m.read_u16(DPC), 19);
        assert_eq!((m.read_u8(DPC + 2), m.read_u8(DPC + 3)), (0, 0));
        assert_eq!((m.read_u32(DPC + 0x0C), m.read_u32(DPC + 0x10)), (ROUTINE as u32, CONTEXT as u32));
        // list entry and arguments are left alone
        assert_eq!((m.read_u32(DPC + 4), m.read_u32(DPC + 0x14)), (0xEEEE_EEEE, 0xEEEE_EEEE));
    }

    #[test]
    fn insert_stores_args_and_returns_the_routine_call() {
        let mut m = FakeMem::default();
        let mut q = HashSet::new();
        initialize_dpc(&mut m, DPC, ROUTINE, CONTEXT);
        // the D3D chain start: arg1 = GPU address of the chain head, arg2 = 0
        let (ret, call) = insert_queue_dpc(&mut m, &mut q, DPC, 0xD011_C280, 0);
        assert_eq!(ret, 1);
        assert_eq!(call, Some(DpcCall { dpc: DPC as u32, routine: ROUTINE as u32, context: CONTEXT as u32, arg1: 0xD011_C280, arg2: 0 }));
        assert_eq!((m.read_u32(DPC + 0x14), m.read_u32(DPC + 0x18)), (0xD011_C280, 0));
        assert_eq!(remove_queue_dpc(&mut q, DPC), 1);
        assert_eq!(remove_queue_dpc(&mut q, DPC), 0);
        // no routine: still queued and returns 1, nothing to run
        let mut m2 = FakeMem::default();
        assert_eq!(insert_queue_dpc(&mut m2, &mut q, 0x5000, 1, 2), (1, None));
    }

    #[test]
    fn impersonation_sets_and_restores_kpcr_state() {
        let mut m = FakeMem::default();
        const KPCR: u64 = 0x9310_2000;
        m.write_u8(KPCR + KPCR_CURRENT_IRQL, 0);
        let prev = begin_impersonation(&mut m, KPCR);
        assert_eq!(prev, Some(0));
        assert_eq!((m.read_u8(KPCR + 0x18), m.read_u32(KPCR + 0x150)), (2, 1));
        // nested insert inside the DPC: no second impersonation
        assert_eq!(begin_impersonation(&mut m, KPCR), None);
        end_impersonation(&mut m, KPCR, prev.unwrap());
        assert_eq!((m.read_u8(KPCR + 0x18), m.read_u32(KPCR + 0x150)), (0, 0));
    }
}
