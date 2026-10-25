//! Emulator application: boot, setup of XUI/GPU/threads/window, then the frame loop.

mod boot;
mod frame_loop;
mod gpu;
mod legacy;
mod threads;
mod xui;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xenon-Emu: Experimental Xbox 360 Emulator in Rust  ");
    println!("========================================================");

    let mut b = boot::boot()?;
    if !b.is_metro {
        legacy::run_init_phases(&mut b.bus, &mut b.kernel, &b.thunk_map, b.real_entry_vaddr, b.vaddr_base, b.stack_top, b.kpcr_base)?;
    }

    let assets = xui::load();

    let (ring_buffer, xenos_gpu) = gpu::setup(b.is_metro);

    // ==========================================================
    // ETAPA 3: Multi-Thread Guest Dispatcher & Live Input Loop
    // ==========================================================
    println!("\n>>> [ETAPA 3] Genuine Dashboard Multi-Thread Interleaved Dispatcher <<<");
    let mut guest_threads = threads::register_initial(&mut b.bus, b.is_metro, b.real_entry_vaddr, b.vaddr_base, b.stack_top, b.kpcr_base, &mut b.next_kpcr_slot)?;
    if b.is_metro {
        // the primary thread is the first thread object (Xenia creates the title's main XThread before it runs)
        let tid = b.kernel.next_thread_id;
        b.kernel.next_thread_id += 1;
        let init = xenon_emu::kernel::objects::KThreadInit {
            thread_id: tid,
            start_address: b.real_entry_vaddr as u32,
            stack_base: b.stack_top as u32,
            stack_limit: (b.stack_top - threads::PRIMARY_STACK_SIZE) as u32,
            creation_flags: 0,
        };
        let (handle, kthread) = b.kernel.create_thread_object(&mut b.bus, &init);
        threads::bind_kthread(&mut b.bus, b.kpcr_base, kthread as u64);
        guest_threads[0].handle = handle;
        println!("[*] Primary thread object: TID=0x{:X}, handle 0x{:08X}, KTHREAD 0x{:08X}", tid, handle, kthread);
    }

    let display = crate::ui::window::open(b.is_metro, &assets.xus_table, &assets.loaded_scenes)?;

    frame_loop::run(b, guest_threads, display, ring_buffer, xenos_gpu)
}
