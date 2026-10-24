//! ETAPA 2: Xenos command processor state and primary ring buffer.

use xenon_emu::gpu::{RingBuffer, XenosGpu};

/// Returns the primary ring buffer (re-bound by the frame loop once the guest initializes its own) and the GPU.
pub fn setup(is_metro: bool) -> (RingBuffer, XenosGpu) {
    println!("
>>> [ETAPA 2] ATI Xenos GPU Command Processor (CP) & Primary Ring Buffer <<<");

    let ring_base: u64 = if is_metro { 0x9010_4000 } else { 0x1000_0000 };
    let ring_size: usize = 64 * 1024;  // 64 KB Primary Ring Buffer
    let ring_buffer = RingBuffer::new(ring_base, ring_size);
    println!("[+] Initialized Xenos Primary Ring Buffer at 0x{:08X} (Size: {} KB)", ring_base, ring_size / 1024);

    (ring_buffer, XenosGpu::new())
}
