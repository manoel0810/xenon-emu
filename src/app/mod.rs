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
    let guest_threads = threads::register_initial(&mut b.bus, b.is_metro, b.real_entry_vaddr, b.vaddr_base, b.stack_top, b.kpcr_base, &mut b.next_kpcr_slot)?;

    let display = crate::ui::window::open(b.is_metro, &assets.xus_table, &assets.loaded_scenes)?;

    frame_loop::run(b, guest_threads, display, ring_buffer, xenos_gpu)
}
