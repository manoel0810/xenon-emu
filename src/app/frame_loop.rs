//! Frame loop: spawns pending guest threads, interleaves their timeslices, feeds the Xenos command processor,
//! fires the VBlank callback and presents the frontbuffer.

use std::collections::HashMap;
use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::diag::{self, profile::Profiler, trace::TrapRing};
use xenon_emu::gpu::display::resolve_path;
use xenon_emu::gpu::{GpuDisplay, RingBuffer, XenosGpu};

use super::boot::Boot;
use super::threads::{begin_dpc_call, callback_stack_top, dynamic_thread_stack_top, end_dpc_call, interrupt_callback_thread, restart_thread, setup_kpcr, DpcFrame, GuestThread, DPC_RETURN_ORDINAL, DYNAMIC_STACK_SIZE};

pub fn run(
    boot: Boot,
    mut guest_threads: Vec<GuestThread>,
    mut display: GpuDisplay,
    mut ring_buffer: RingBuffer,
    mut xenos_gpu: XenosGpu,
) -> Result<(), Box<dyn std::error::Error>> {
    let Boot { mut bus, mut kernel, thunk_map, is_metro, mut next_kpcr_slot, .. } = boot;

    let diag = diag::config();
    let initial_fb = kernel.active_frontbuffer();
    println!("[*] Guest VRAM Framebuffer Ativo inicial: 0x{:08X}", initial_fb);
    println!("[*] Fonte de Vídeo inicial: {} (tecla [B] alterna; XENON_FB_SOURCE=reference inicia no dump)", if diag.fb_source_reference { "DUMP DE REFERÊNCIA" } else { "VRAM Guest Ao Vivo" });

    let max_frames: Option<u64> = diag.max_frames;
    // Trap diagnostics: XENON_TRAP_TRACE keeps a ring of the last executed instructions of the
    // Metro primary thread and dumps it on trap; XENON_NO_RESTART stops the thread instead of restarting it.
    let trap_trace = diag.trap_trace;
    let no_restart = diag.no_restart;
    let trap_full = diag.trap_full;
    // Restart policy: the Metro primary thread runs the CRT startup once; restarting it re-runs startup
    // over already-initialised globals (double InsertTailList -> self-linked node). Only restart it when asked.
    let restart_main = diag.restart_main;
    // First-run register snapshot per thread: (SP, r3, r4, r5, r13), used to give restarts a clean stack/arguments
    let mut thread_init: HashMap<String, (u64, u64, u64, u64, u64)> = HashMap::new();
    // XENON_WATCH_DUMP_HIT=<n>: dump the instruction ring right after the n-th watchpoint hit (needs XENON_TRAP_TRACE=1)
    let watch_dump_hit: Option<u64> = diag.watch_dump_hit;
    let mut watch_dump_done = false;
    let dump_ring_frame: Option<u64> = diag.dump_ring_frame;
    let heartbeat = diag.heartbeat;
    let gpu_mmio_log = diag.gpu_mmio_log;
    let syscall_log = diag.syscall_log;
    let syscall_summary = diag.syscall_summary;
    let watchdog = diag.watchdog;
    if watchdog {
        diag::watchdog::spawn();
    }
    // XENON_PC_HOOK=0xADDR,0xADDR: dump registers whenever the Metro primary thread reaches these PCs
    let hook_all_threads = diag.pc_hook_all;
    let mut pc_hook_hits: u64 = 0;
    let pc_hook_min_frame: u64 = diag.pc_hook_min_frame;
    let pc_hook_limit: u64 = diag.pc_hook_limit;
    let pc_hooks: &[u64] = &diag.pc_hooks;
    let mut trap_ring = TrapRing::new();
    let pc_profile = diag.pc_profile;
    let mut profiler = Profiler::new();

    let mut next_callback_slot: u64 = 0;
    let mut dynamic_threads_created: u64 = 0;
    // DPC routines running inline on a guest thread (KeInsertQueueDpc), by thread name
    let mut dpc_frames: HashMap<String, Vec<DpcFrame>> = HashMap::new();
    let mut frame = 0u64;
    while display.is_open() {
        frame += 1;
        diag::watchdog::set_frame(frame);
        if let Some(mf) = max_frames {
            if frame > mf {
                println!("[*] Reached XENON_MAX_FRAMES limit ({} frames) - terminating loop cleanly.", mf);
                break;
            }
        }
        display.handle_input();

        // 1. Process any pending dynamic thread creations from ExCreateThread
        while let Some(pending) = kernel.pending_threads.pop() {
            let thread_stack = dynamic_thread_stack_top(dynamic_threads_created);
            dynamic_threads_created += 1;
            let _ = bus.write_u32(thread_stack - 8, 0x80001000);
            let _ = bus.write_u32(thread_stack + 0x100 - 8, 0x80001000);
            let routine = (pending.start_routine & 0xFFFF_FFFF) as u64;
            let mut thread_cpu = Cpu::new(routine);
            thread_cpu.regs.gpr[1] = thread_stack;
            thread_cpu.regs.gpr[3] = pending.start_context;
            thread_cpu.regs.gpr[13] = { let s = 8 + (next_kpcr_slot % 40); next_kpcr_slot += 1; setup_kpcr(&mut bus, s, thread_stack, thread_stack - (DYNAMIC_STACK_SIZE - 0x1000)) };
            thread_cpu.regs.lr = 0x80001000;
            println!("  [SCHEDULER] Spawning dynamic guest thread TID=0x{:X} (@0x{:08X})",
                pending.tid, routine
            );
            let is_worker_or_render = true; // Dashboard dynamic worker/UI threads should run continuously
            guest_threads.push(GuestThread {
                name: format!("Dynamic Thread 0x{:X}", pending.tid),
                cpu: thread_cpu,
                entry_pc: routine,
                active: true,
                total_steps: 0,
                continuous: is_worker_or_render,
            });
        }

        // 2. Feed live unified gamepad state (USB controller or keyboard) into HLE Kernel
        let gp_state = display.get_current_gamepad_state();
        kernel.update_gamepad(gp_state);

        // 3. Interleave continuous guest execution of genuine dashboard threads
        let active_threads = guest_threads.iter().filter(|t| t.active).count() as u64;
        bus.timebase.begin_frame(frame, active_threads * 25000);
        for thread in &mut guest_threads {
            if !thread.active {
                continue;
            }
            if heartbeat {
                use std::io::Write;
                eprintln!("[HB] frame {} thread '{}' pc=0x{:08X} lr=0x{:08X}", frame, thread.name, thread.cpu.regs.pc & 0xFFFF_FFFF, thread.cpu.regs.lr & 0xFFFF_FFFF);
                let _ = std::io::stderr().flush();
            }
            bus.current_thread.clear();
            bus.current_thread.push_str(&thread.name);
            if !thread_init.contains_key(&thread.name) {
                let r = &thread.cpu.regs;
                thread_init.insert(thread.name.clone(), (r.gpr[1], r.gpr[3], r.gpr[4], r.gpr[5], r.gpr[13]));
            }
            // Execute a timeslice of up to 25,000 instructions per thread per frame
            let slice_start = std::time::Instant::now();
            for step_i in 0..25000u32 {
                if watchdog && (step_i & 0x1FF) == 0 && slice_start.elapsed().as_secs() >= 3 {
                    eprintln!("[WATCHDOG] slice > 3s: frame {} thread '{}' step {} pc=0x{:08X} lr=0x{:08X} ctr=0x{:X} r1=0x{:X} r3=0x{:X} r4=0x{:X} r5=0x{:X}",
                        frame, thread.name, step_i, thread.cpu.regs.pc & 0xFFFF_FFFF, thread.cpu.regs.lr & 0xFFFF_FFFF, thread.cpu.regs.ctr,
                        thread.cpu.regs.gpr[1], thread.cpu.regs.gpr[3], thread.cpu.regs.gpr[4], thread.cpu.regs.gpr[5]);
                    std::process::exit(86);
                }
                let current_pc = thread.cpu.regs.pc;
                if watchdog {
                    diag::watchdog::on_step(current_pc, thread.cpu.regs.lr);
                }
                bus.current_pc = current_pc;
                bus.current_lr = thread.cpu.regs.lr;
                bus.current_sp = thread.cpu.regs.gpr[1];
                match bus.read_u32(current_pc) {
                    Ok(op_raw) => {
                        if !pc_hooks.is_empty() && (hook_all_threads || thread.name.starts_with("Metro")) && pc_hooks.contains(&(current_pc & 0xFFFF_FFFF)) && frame >= pc_hook_min_frame {
                            pc_hook_hits += 1;
                            if pc_hook_hits <= pc_hook_limit {
                                diag::hooks::dump(diag, &bus, &thread.cpu.regs, pc_hook_hits, frame, &thread.name, current_pc);
                            }
                        }
                        if trap_trace && thread.name.starts_with("Metro") {
                            trap_ring.record(current_pc, op_raw, &thread.cpu.regs);
                        }
                        match thread.cpu.step(&mut bus) {
                            Ok(StepResult::Ok) => {
                                thread.total_steps += 1;
                                if let Some(h) = watch_dump_hit {
                                    if !watch_dump_done && bus.watch_hits >= h {
                                        watch_dump_done = true;
                                        trap_ring.dump_watch_hit(bus.watch_hits, current_pc, diag.dump_ring_n);
                                    }
                                }
                                if pc_profile && !thread.name.starts_with("VBlank") && !thread.name.starts_with("CP Interrupt") {
                                    profiler.record_pc(&thread.name, current_pc);
                                }
                            }
                            Ok(StepResult::Syscall { lev: _, pc }) => {
                                let thunk = thunk_map.get(&pc).or_else(|| thunk_map.get(&(pc & 0xFFFF_FFFF)));
                                if pc_profile {
                                    if let Some((l, o)) = thunk {
                                        profiler.record_syscall(&thread.name, l, *o);
                                    }
                                }
                                if let Some((_, ordinal)) = thunk {
                                    if *ordinal == DPC_RETURN_ORDINAL {
                                        // a DPC routine returned: resume the KeInsertQueueDpc caller
                                        end_dpc_call(&mut thread.cpu, &mut bus, dpc_frames.entry(thread.name.clone()).or_default());
                                        continue;
                                    }
                                }
                                if let Some((lib, ordinal)) = thunk {
                                    let is_term = *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe");
                                    let is_yield = (*ordinal == 0x0101 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x002B && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x0057 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00B0 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00AF && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00FD && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00FE && lib.eq_ignore_ascii_case("xboxkrnl.exe"));

                                    let args = [
                                        thread.cpu.regs.gpr[3], thread.cpu.regs.gpr[4], thread.cpu.regs.gpr[5],
                                        thread.cpu.regs.gpr[6], thread.cpu.regs.gpr[7], thread.cpu.regs.gpr[8],
                                        thread.cpu.regs.gpr[9], thread.cpu.regs.gpr[10],
                                    ];
                                    if syscall_summary {
                                        profiler.record_call(&thread.name, lib, *ordinal, thread.cpu.regs.lr, frame, &args);
                                    }
                                    if syscall_log {
                                        use std::io::Write;
                                        eprintln!("[SYS] f{} {} {}!0x{:04X} a=[{:X},{:X},{:X},{:X},{:X},{:X}] lr=0x{:08X}", frame, thread.name, lib, ordinal, args[0], args[1], args[2], args[3], args[4], args[5], thread.cpu.regs.lr & 0xFFFF_FFFF);
                                        let _ = std::io::stderr().flush();
                                    }
                                    if watchdog {
                                        diag::watchdog::enter_syscall(lib, *ordinal, &args);
                                    }
                                    let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                                    if watchdog { diag::watchdog::leave_syscall(); }
                                    thread.cpu.regs.gpr[3] = ret;
                                    thread.cpu.regs.pc = thread.cpu.regs.lr;
                                    if let Some(call) = kernel.pending_dpc_call.take() {
                                        // KeInsertQueueDpc: run the DPC routine inline on this thread before returning
                                        begin_dpc_call(&mut thread.cpu, &mut bus, dpc_frames.entry(thread.name.clone()).or_default(), call);
                                    }

                                    if is_term {
                                        if thread.continuous && no_restart {
                                            println!("  [NO_RESTART] thread '{}' called ExTerminateThread; stopping", thread.name);
                                            thread.continuous = false;
                                            thread.active = false;
                                            break;
                                        }
                                        if thread.continuous && thread.name.starts_with("Metro") && !restart_main {
                                            println!("  [THREAD EXIT] '{}' called ExTerminateThread at frame {} (pc=0x{:08X}); not restarting (set XENON_RESTART_MAIN=1 to restart)",
                                                thread.name, frame, current_pc);
                                            thread.continuous = false;
                                            thread.active = false;
                                            break;
                                        }
                                        if thread.continuous {
                                            if diag.watch_set {
                                                println!("  [RESTART:TERM] thread '{}' frame {} called ExTerminateThread (pc=0x{:08X}, SP=0x{:08X}) -> restarting at entry 0x{:08X}",
                                                    thread.name, frame, current_pc, thread.cpu.regs.gpr[1], thread.entry_pc);
                                            }
                                            restart_thread(&mut thread.cpu, thread.entry_pc, thread_init.get(&thread.name));
                                            break;
                                        } else {
                                            thread.active = false;
                                            break;
                                        }
                                    }
                                    if is_yield {
                                        // Yield timeslice to next thread or frame
                                        break;
                                    }
                                } else {
                                    thread.active = false;
                                    break;
                                }
                            }
                            Ok(StepResult::Halted) => {
                                thread.active = false;
                                break;
                            }
                            Err(e) => {
                                if thread.continuous {
                                    if trap_trace && thread.name.starts_with("Metro") {
                                        trap_ring.dump_trap(&e, thread.cpu.regs.lr, thread.cpu.regs.ctr, trap_full);
                                    }
                                    if no_restart {
                                        println!("  [NO_RESTART] stopping thread '{}' after first trap", thread.name);
                                        thread.continuous = false;
                                        thread.active = false;
                                        break;
                                    }
                                    if thread.name.starts_with("Metro") && !restart_main {
                                        println!("  [THREAD TRAP] '{}' trapped at frame {} pc=0x{:08X} SP=0x{:08X}: {} -- thread stopped (set XENON_RESTART_MAIN=1 to restart)",
                                            thread.name, frame, current_pc, thread.cpu.regs.gpr[1], e);
                                        thread.continuous = false;
                                        thread.active = false;
                                        break;
                                    }
                                    if diag.watch_set {
                                        println!("  [RESTART:ERR] thread '{}' frame {} trapped at pc=0x{:08X} SP=0x{:08X}: {} -> restarting at entry 0x{:08X}",
                                            thread.name, frame, current_pc, thread.cpu.regs.gpr[1], e, thread.entry_pc);
                                    }
                                    restart_thread(&mut thread.cpu, thread.entry_pc, thread_init.get(&thread.name));
                                } else {
                                    thread.active = false;
                                }
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        thread.active = false;
                        break;
                    }
                }
            }
            if let Some(df) = dump_ring_frame {
                if frame == df && thread.name.starts_with("Metro") && !trap_ring.is_empty() {
                    trap_ring.dump_frame(frame, diag.dump_ring_n);
                }
            }
        }

        // Retain only active or continuous threads to prevent vector bloat
        guest_threads.retain(|t| t.active || t.continuous);

        // 4. Synchronize & Execute PM4 packets in Primary Ring Buffer with Xenos GPU CP
        if kernel.ring_buffer_ptr != 0 && kernel.ring_buffer_ptr != ring_buffer.base_addr {
            let sz = if kernel.ring_buffer_size <= 32 {
                let comp = (1usize << kernel.ring_buffer_size) * 4;
                comp.max(32 * 1024)
            } else if kernel.ring_buffer_size != 0 {
                kernel.ring_buffer_size as usize
            } else {
                32 * 1024
            };
            println!("  [GPU] Re-binding RingBuffer to guest configured address 0x{:08X} (Size: {} KB)", kernel.ring_buffer_ptr, sz / 1024);
            ring_buffer = RingBuffer::new(kernel.ring_buffer_ptr, sz);
        }

        // The dash signals new commands by storing the ring write pointer (in dwords) to CP_RB_WPTR in the GPU MMIO window.
        // Use it when the guest has ever written it; otherwise fall back to scanning the ring for the last non-zero word.
        if let Some(w) = bus.cp_rb_wptr {
            if gpu_mmio_log && w as usize != ring_buffer.wptr {
                println!("  [GPU MMIO] frame {} CP_RB_WPTR = {} (was {}) ring={:#X} size={} dwords, total mmio writes={}", frame, w, ring_buffer.wptr, ring_buffer.base_addr, ring_buffer.num_dwords, bus.gpu_mmio_writes);
            }
            ring_buffer.wptr = (w as usize) % ring_buffer.num_dwords.max(1);
        } else {
            ring_buffer.sync_wptr_from_memory(&bus);
        }
        let executed_pkts = xenos_gpu
            .execute_ring_with_writeback(&mut bus, &mut ring_buffer, kernel.rptr_writeback_ptr)
            .unwrap_or(0);

        if executed_pkts > 0 {
            println!(
                "  [GPU CP] Frame {:05} | Processed {} PM4 packets (Total: {}, Draw Calls: {}, Primitives: {}) | WPTR: {}, RPTR: {}",
                frame, executed_pkts, xenos_gpu.packets_executed, xenos_gpu.draw_calls, xenos_gpu.primitives_drawn, ring_buffer.wptr, ring_buffer.rptr
            );
        }

        // 5. Frontbuffer to present: the one passed to the guest's last VdSwap (the guest calls VdSwap itself; the
        // swap packets it writes are executed by the CP as XE_SWAP).
        let frontbuffer = kernel.active_frontbuffer();

        // 5b. Graphics interrupts (VdSetGraphicsInterruptCallback, #0x01D5), as Xenia: every vblank advances the GPU
        // counter and calls callback(0, context) on cpu 2; each PM4 INTERRUPT queued a callback(1, context) on its cpu.
        xenos_gpu.mark_vblank();
        let cp_interrupts = std::mem::take(&mut xenos_gpu.pending_cp_interrupts);
        if kernel.graphics_interrupt_callback != 0 {
            let sources = std::iter::once((0u64, 2u8)).chain(cp_interrupts.iter().map(|&n| (1u64, n as u8)));
            for (source, cpu) in sources {
                let name = if source == 0 { format!("VBlank Interrupt Callback #{}", frame) } else { format!("CP Interrupt Callback #{}.{}", frame, cpu) };
                let slot = 48 + (next_callback_slot % 16);
                next_callback_slot += 1;
                guest_threads.push(interrupt_callback_thread(
                    &mut bus,
                    kernel.graphics_interrupt_callback,
                    kernel.graphics_interrupt_context,
                    source,
                    cpu,
                    callback_stack_top(slot - 48),
                    slot,
                    name,
                ));
            }
        }

        // 6. Direct Presentation: Boot Animation -> Direct ATI Xenos GPU Framebuffer
        display.present_guest_memory_framebuffer(&bus, frontbuffer, &xenos_gpu.color_buffer);
        display.update()?;

        if frame == 70 {
            let _ = display.save_bmp(resolve_path("bootanim_ribbons.bmp"));
        } else if frame == 180 {
            let _ = display.save_bmp(resolve_path("bootanim_sphere.bmp"));
        } else if frame == 290 {
            let out_name = if is_metro { resolve_path("dash_17559_framebuffer.bmp") } else { resolve_path("dash_framebuffer.bmp") };
            let _ = display.save_bmp(&out_name);
        }

        if frame % 180 == 0 {
            let total_guest_steps: u64 = guest_threads.iter().map(|t| t.total_steps).sum();
            println!(
                "  [GPU] Frame {:05} | FrontBuffer: 0x{:08X} | Ring Executed: {} pkts | 60 FPS | Guest Code: {} insns",
                frame, frontbuffer, executed_pkts, total_guest_steps
            );
            for t in &guest_threads {
                println!(
                    "    -> [THREAD] \"{}\" | PC: 0x{:08X} | LR: 0x{:08X} | Steps: {} | Active: {} | Cont: {}",
                    t.name, t.cpu.regs.pc, t.cpu.regs.lr, t.total_steps, t.active, t.continuous
                );
            }
        }
    }

    if syscall_summary {
        profiler.print_syscall_summary();
    }

    if pc_profile {
        for t in &guest_threads {
            if t.name.starts_with("Metro") {
                let r = &t.cpu.regs.gpr;
                println!("[PROFILE] Primary regs: pc=0x{:08X} r3=0x{:08X} r11=0x{:08X} r28=0x{:08X} r30=0x{:08X} r31=0x{:08X}",
                    t.cpu.regs.pc, r[3], r[11], r[28], r[30], r[31]);
                let mut node = r[30] & 0xFFFF_FFFF;
                for i in 0..8 {
                    let next = bus.read_u32(node).unwrap_or(0);
                    let prev = bus.read_u32(node + 4).unwrap_or(0);
                    let cb = bus.read_u32(node + 8).unwrap_or(0);
                    println!("  node[{}] @0x{:08X}: next=0x{:08X} prev=0x{:08X} +8=0x{:08X}", i, node, next, prev, cb);
                    node = next as u64;
                }
                let h = r[31] & 0xFFFF_FFFF;
                println!("  head @0x{:08X}: next=0x{:08X} prev=0x{:08X}", h, bus.read_u32(h).unwrap_or(0), bus.read_u32(h + 4).unwrap_or(0));
            }
        }
        profiler.print_hot();
    }

    crate::ui::window::finish(&display, is_metro)
}
