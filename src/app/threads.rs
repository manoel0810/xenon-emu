//! Guest threads run by the frame loop: per-thread KPCR, initial thread set and restart helper.

use xenon_emu::cpu::Cpu;
use xenon_emu::kernel::dpc::{self, DpcCall};
use xenon_emu::memory::MemoryBus;

/// Guest address of the DPC return trampoline (`sc; blr`), registered in the thunk map as a pseudo export.
pub const DPC_RETURN_TRAMPOLINE: u64 = 0x80001010;
/// Pseudo ordinal of the DPC return trampoline (not a real xboxkrnl export; never reaches the kernel).
pub const DPC_RETURN_ORDINAL: u16 = 0xFFFE;

/// A DPC routine running on a guest thread: where to resume and the IRQL to restore (None = nested DPC).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DpcFrame {
    pub return_pc: u64,
    pub previous_irql: Option<u8>,
}

/// Runs a DPC inline on the thread that called KeInsertQueueDpc (Xenia KeInsertQueueDpc_entry): the thread's
/// `pc` is already the return address; it now calls routine(dpc, context, arg1, arg2) at DISPATCH_LEVEL and comes
/// back through DPC_RETURN_TRAMPOLINE.
pub fn begin_dpc_call(cpu: &mut Cpu, bus: &mut MemoryBus, frames: &mut Vec<DpcFrame>, call: DpcCall) {
    let previous_irql = dpc::begin_impersonation(bus, cpu.regs.gpr[13]);
    frames.push(DpcFrame { return_pc: cpu.regs.pc, previous_irql });
    cpu.regs.gpr[3] = call.dpc as u64;
    cpu.regs.gpr[4] = call.context as u64;
    cpu.regs.gpr[5] = call.arg1 as u64;
    cpu.regs.gpr[6] = call.arg2 as u64;
    cpu.regs.lr = DPC_RETURN_TRAMPOLINE;
    cpu.regs.pc = call.routine as u64;
}

/// The DPC routine returned to the trampoline: restore IRQL/dpc_active and return 1 from KeInsertQueueDpc.
/// Returns false if the thread had no DPC in progress.
pub fn end_dpc_call(cpu: &mut Cpu, bus: &mut MemoryBus, frames: &mut Vec<DpcFrame>) -> bool {
    let Some(frame) = frames.pop() else { return false };
    if let Some(irql) = frame.previous_irql {
        dpc::end_impersonation(bus, cpu.regs.gpr[13], irql);
    }
    cpu.regs.gpr[3] = 1;
    cpu.regs.pc = frame.return_pc;
    cpu.regs.lr = frame.return_pc;
    true
}

pub struct GuestThread {
    pub name: String,
    pub cpu: Cpu,
    pub entry_pc: u64,
    pub active: bool,
    pub total_steps: u64,
    pub continuous: bool,
}

/// Per-thread KPCR/KTHREAD ("r13 block"). On the real console every thread has its own, so thread-local state such as
/// the Win32 last-error value (KTHREAD+0x160) is NOT shared. Previously all guest threads shared one KPCR at 0x7FE00000,
/// so a worker's SetLastError(0x102 / WAIT_TIMEOUT) leaked into the primary thread's GetLastError() and made the dash's
/// service initialisation fail. The blocks live in RAM above the dash image (outside the virtual-alloc window).
const KPCR_AREA: u64 = 0x9310_0000;
const KPCR_STRIDE: u64 = 0x2000; // 0x1000 KPCR + 0x1000 TLS

pub fn setup_kpcr(bus: &mut MemoryBus, slot: u64, stack_base: u64, stack_limit: u64) -> u64 {
    let kpcr = KPCR_AREA + slot * KPCR_STRIDE;
    let tls = kpcr + 0x1000;
    for off in (0..KPCR_STRIDE).step_by(4) {
        let _ = bus.write_u32(kpcr + off, 0);
    }
    let _ = bus.write_u32(kpcr + 0x00, tls as u32);              // tls_ptr
    let _ = bus.write_u32(kpcr + 0x30, kpcr as u32);             // pcr_ptr
    let _ = bus.write_u32(kpcr + 0x70, stack_base as u32);       // stack_base_ptr
    let _ = bus.write_u32(kpcr + 0x74, stack_limit as u32);      // stack_end_ptr
    let _ = bus.write_u32(kpcr + 0x100, (kpcr + 0x200) as u32);  // current_thread (KTHREAD)
    let _ = bus.write_u8(kpcr + 0x10C, 0);                       // current_cpu
    let _ = bus.write_u32(kpcr + 0x14C, (slot + 1) as u32);      // thread_id
    let _ = bus.write_u32(kpcr + 0x150, 0);                      // dpc_active
    let _ = bus.write_u32(kpcr + 0x160, 0);                      // last_error
    kpcr
}

pub fn restart_thread(cpu: &mut Cpu, entry: u64, init: Option<&(u64, u64, u64, u64, u64)>) {
    cpu.regs.pc = entry;
    cpu.regs.lr = 0x80001000;
    cpu.regs.ctr = 0;
    if let Some(&(sp, r3, r4, r5, r13)) = init {
        cpu.regs.gpr[1] = sp;
        cpu.regs.gpr[3] = r3;
        cpu.regs.gpr[4] = r4;
        cpu.regs.gpr[5] = r5;
        cpu.regs.gpr[13] = r13;
    }
}

/// Graphics interrupt callback (VdSetGraphicsInterruptCallback), invoked like Xenia's
/// KernelState::EmulateCPInterruptDPC: callback(source, context) with `cpu` as the active CPU (KPCR+0x10C).
/// source 0 = vblank (Xenia MarkVblank uses cpu 2), 1 = CP interrupt from a PM4 INTERRUPT packet.
/// Runs as a short-lived guest thread on its own stack and KPCR slot.
pub fn interrupt_callback_thread(
    bus: &mut MemoryBus,
    routine: u64,
    context: u64,
    source: u64,
    cpu: u8,
    stack: u64,
    kpcr_slot: u64,
    name: String,
) -> GuestThread {
    let routine = routine & 0xFFFF_FFFF;
    let _ = bus.write_u32(stack - 8, 0x80001000);
    let _ = bus.write_u32(stack + 0x100 - 8, 0x80001000);
    let mut cb_cpu = Cpu::new(routine);
    cb_cpu.regs.gpr[1] = stack;
    cb_cpu.regs.gpr[3] = source;
    cb_cpu.regs.gpr[4] = context;
    let kpcr = setup_kpcr(bus, kpcr_slot, stack, stack - 0x20000);
    let _ = bus.write_u8(kpcr + 0x10C, cpu); // current_cpu: the dash clears this CPU's bit for CP interrupts
    cb_cpu.regs.gpr[13] = kpcr;
    cb_cpu.regs.lr = 0x80001000;
    GuestThread { name, cpu: cb_cpu, entry_pc: routine, active: true, total_steps: 0, continuous: false }
}

/// Registers the threads that exist before the first frame (Metro: only the primary thread).
pub fn register_initial(
    bus: &mut MemoryBus,
    is_metro: bool,
    real_entry_vaddr: u64,
    vaddr_base: u64,
    stack_top: u64,
    kpcr_base: u64,
    next_kpcr_slot: &mut u64,
) -> Result<Vec<GuestThread>, Box<dyn std::error::Error>> {
    let mut guest_threads = Vec::new();

    if is_metro {
        println!("[*] Registering Metro 17559 Primary Thread (Entry: 0x{:08X}, SP: 0x{:08X})...", real_entry_vaddr, stack_top);
        let mut metro_cpu = Cpu::new(real_entry_vaddr);
        metro_cpu.regs.gpr[1] = stack_top;
        metro_cpu.regs.gpr[3] = vaddr_base; // r3: hModule (0x92000000)
        metro_cpu.regs.gpr[4] = 1;          // r4: DLL_PROCESS_ATTACH (1)
        metro_cpu.regs.gpr[5] = 0;          // r5: lpReserved (0)
        metro_cpu.regs.gpr[13] = kpcr_base; // r13: KPCR
        metro_cpu.regs.lr = 0x80001000;     // LR: ExTerminateThread stub
        bus.write_u32(stack_top - 8, 0x80001000)?;
        bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;
        guest_threads.push(GuestThread {
            name: "Metro 17559 Primary Thread".to_string(),
            cpu: metro_cpu,
            entry_pc: real_entry_vaddr,
            active: true,
            total_steps: 0,
            continuous: true,
        });
    } else {
        // 1. Dashboard Main Controller / Event Pump (@ 0x921C9830 from live console)
        let stack_top_ctrl = stack_top - 0xA0000;
        bus.write_u32(stack_top_ctrl - 8, 0x80001000)?;
        bus.write_u32(stack_top_ctrl + 0x100 - 8, 0x80001000)?;
        let mut ctrl_cpu = Cpu::new(0x921C9830);
        ctrl_cpu.regs.gpr[1] = stack_top_ctrl;
        ctrl_cpu.regs.gpr[3] = 0;
        ctrl_cpu.regs.gpr[4] = 0;
        ctrl_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_ctrl, stack_top_ctrl - 0x20000) };
        ctrl_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Main Controller & Event Pump".to_string(),
            cpu: ctrl_cpu,
            entry_pc: 0x921C9830,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 2. XUI Presentation & Scene Graph Thread (@ 0x9281FBC0 from live console)
        let stack_top_xui = stack_top - 0xC0000;
        bus.write_u32(stack_top_xui - 8, 0x80001000)?;
        bus.write_u32(stack_top_xui + 0x100 - 8, 0x80001000)?;
        let mut xui_cpu = Cpu::new(0x9281FBC0);
        xui_cpu.regs.gpr[1] = stack_top_xui;
        xui_cpu.regs.gpr[3] = 0;
        xui_cpu.regs.gpr[4] = 0;
        xui_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_xui, stack_top_xui - 0x20000) };
        xui_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "XUI Presentation Engine".to_string(),
            cpu: xui_cpu,
            entry_pc: 0x9281FBC0,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 3. Direct3D 9 / GPU Driver Loop (@ 0x92185BC0 from live console)
        let stack_top_gpu = stack_top - 0xE0000;
        bus.write_u32(stack_top_gpu - 8, 0x80001000)?;
        bus.write_u32(stack_top_gpu + 0x100 - 8, 0x80001000)?;
        let mut gpu_cpu = Cpu::new(0x92185BC0);
        gpu_cpu.regs.gpr[1] = stack_top_gpu;
        gpu_cpu.regs.gpr[3] = 0;
        gpu_cpu.regs.gpr[4] = 0;
        gpu_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_gpu, stack_top_gpu - 0x20000) };
        gpu_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Direct3D 9 GPU Driver".to_string(),
            cpu: gpu_cpu,
            entry_pc: 0x92185BC0,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 4. Dashboard State Machine Thread (@ 0x92261CE8 from live console)
        let stack_top_sm = stack_top - 0x100000;
        bus.write_u32(stack_top_sm - 8, 0x80001000)?;
        bus.write_u32(stack_top_sm + 0x100 - 8, 0x80001000)?;
        let mut sm_cpu = Cpu::new(0x92261CE8);
        sm_cpu.regs.gpr[1] = stack_top_sm;
        sm_cpu.regs.gpr[3] = 0;
        sm_cpu.regs.gpr[4] = 0;
        sm_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_sm, stack_top_sm - 0x20000) };
        sm_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Dashboard State Machine".to_string(),
            cpu: sm_cpu,
            entry_pc: 0x92261CE8,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 5. Background Resource & Stream Worker (@ 0x92267C58 from live console)
        let stack_top_res = stack_top - 0x120000;
        bus.write_u32(stack_top_res - 8, 0x80001000)?;
        bus.write_u32(stack_top_res + 0x100 - 8, 0x80001000)?;
        let mut res_cpu = Cpu::new(0x92267C58);
        res_cpu.regs.gpr[1] = stack_top_res;
        res_cpu.regs.gpr[3] = 0;
        res_cpu.regs.gpr[4] = 0;
        res_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_res, stack_top_res - 0x20000) };
        res_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Resource & Stream Worker".to_string(),
            cpu: res_cpu,
            entry_pc: 0x92267C58,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 6. Content & Tile Initializer Worker (@ 0x92107280)
        let stack_top_worker = stack_top - 0x140000;
        bus.write_u32(stack_top_worker - 8, 0x80001000)?;
        bus.write_u32(stack_top_worker + 0x320 - 8, 0x80001000)?;
        let mut worker_cpu = Cpu::new(0x92107280);
        worker_cpu.regs.gpr[1] = stack_top_worker;
        worker_cpu.regs.gpr[3] = 0;
        worker_cpu.regs.gpr[4] = 0;
        worker_cpu.regs.gpr[13] = { let s = *next_kpcr_slot; *next_kpcr_slot += 1; setup_kpcr(bus, s, stack_top_worker, stack_top_worker - 0x20000) };
        worker_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Content & Tile Initializer".to_string(),
            cpu: worker_cpu,
            entry_pc: 0x92107280,
            active: true,
            total_steps: 0,
            continuous: true,
        });
    }

    println!("[+] Registered {} authentic Dashboard threads into 60 FPS guest scheduler:", guest_threads.len());
    for t in &guest_threads {
        println!("  - Thread '{}' -> Entry PC: 0x{:08X}, SP: 0x{:08X}", t.name, t.cpu.regs.pc, t.cpu.regs.gpr[1]);
    }
    Ok(guest_threads)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpc_runs_inline_and_returns_to_the_caller() {
        let mut bus = MemoryBus::new();
        let kpcr = setup_kpcr(&mut bus, 5, 0x92E0_0000, 0x92DE_0000);
        let mut cpu = Cpu::new(0);
        cpu.regs.gpr[13] = kpcr;
        cpu.regs.gpr[1] = 0x92DF_F000;
        // state right after the KeInsertQueueDpc syscall: pc = caller's return address
        cpu.regs.pc = 0x921B_13E0;
        cpu.regs.lr = 0x921B_13E0;
        let mut frames = Vec::new();
        let call = DpcCall { dpc: 0x4000_51C4, routine: 0x921C_89B0, context: 0x4000_51A4, arg1: 0xD011_C280, arg2: 0 };
        begin_dpc_call(&mut cpu, &mut bus, &mut frames, call);
        assert_eq!((cpu.regs.pc, cpu.regs.lr), (0x921C_89B0, DPC_RETURN_TRAMPOLINE));
        assert_eq!((cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5], cpu.regs.gpr[6]), (0x4000_51C4, 0x4000_51A4, 0xD011_C280, 0));
        assert_eq!(cpu.regs.gpr[1], 0x92DF_F000, "the routine runs on the caller's stack");
        assert_eq!((bus.read_u8(kpcr + 0x18).unwrap(), bus.read_u32(kpcr + 0x150).unwrap()), (2, 1));
        // the routine returns through the trampoline
        assert!(end_dpc_call(&mut cpu, &mut bus, &mut frames));
        assert_eq!((cpu.regs.pc, cpu.regs.lr, cpu.regs.gpr[3]), (0x921B_13E0, 0x921B_13E0, 1));
        assert_eq!((bus.read_u8(kpcr + 0x18).unwrap(), bus.read_u32(kpcr + 0x150).unwrap()), (0, 0));
        assert!(!end_dpc_call(&mut cpu, &mut bus, &mut frames));
    }

    #[test]
    fn nested_dpc_keeps_the_outer_impersonation() {
        let mut bus = MemoryBus::new();
        let kpcr = setup_kpcr(&mut bus, 6, 0x92E0_0000, 0x92DE_0000);
        let mut cpu = Cpu::new(0x1000);
        cpu.regs.gpr[13] = kpcr;
        let mut frames = Vec::new();
        let call = DpcCall { dpc: 1, routine: 0x2000, context: 0, arg1: 0, arg2: 0 };
        begin_dpc_call(&mut cpu, &mut bus, &mut frames, call);
        cpu.regs.pc = 0x2040; // the routine inserts another DPC
        begin_dpc_call(&mut cpu, &mut bus, &mut frames, DpcCall { routine: 0x3000, ..call });
        assert!(end_dpc_call(&mut cpu, &mut bus, &mut frames));
        assert_eq!((cpu.regs.pc, bus.read_u32(kpcr + 0x150).unwrap()), (0x2040, 1), "still inside the outer DPC");
        assert!(end_dpc_call(&mut cpu, &mut bus, &mut frames));
        assert_eq!((cpu.regs.pc, bus.read_u32(kpcr + 0x150).unwrap()), (0x1000, 0));
    }

    #[test]
    fn interrupt_callback_gets_source_then_context_and_active_cpu() {
        let mut bus = MemoryBus::new();
        // the dash's callback is 0x921AFCA0(r3 = source, r4 = device)
        let t = interrupt_callback_thread(&mut bus, 0x921A_FCA0, 0x4000_2400, 1, 3, 0x92E3_8000, 60, "cb".into());
        assert_eq!((t.cpu.regs.pc, t.cpu.regs.gpr[3], t.cpu.regs.gpr[4]), (0x921A_FCA0, 1, 0x4000_2400));
        assert_eq!(t.cpu.regs.gpr[13], KPCR_AREA + 60 * KPCR_STRIDE);
        assert_eq!(bus.read_u8(t.cpu.regs.gpr[13] + 0x10C).unwrap(), 3);
        assert_eq!((t.cpu.regs.lr, t.cpu.regs.gpr[1], t.continuous), (0x80001000, 0x92E3_8000, false));
    }
}
