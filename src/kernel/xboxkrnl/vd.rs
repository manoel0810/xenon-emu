//! xboxkrnl.exe HLE: Vd*: video driver (ring buffer, display, swap, interrupts).

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_vd(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x01B1..0x01B9: Video Display (Vd*) Handlers
            0x01B1 | 0x01B3 | 0x01B4 | 0x01B5 | 0x01B7 | 0x01B8 | 0x01B9 => 0,

            // 0x01B6: VdEnableRingBufferRPtrWriteBack (pWriteBackAddress, frequency)
            0x01B6 => {
                if let Some(&ptr) = args.first() {
                    self.rptr_writeback_ptr = ptr;
                    println!("    -> VdEnableRingBufferRPtrWriteBack: Address=0x{:08X}", ptr);
                }
                0
            }

            // 0x01BA: VdGetCurrentDisplayInformation
            0x01BA => {
                if let Some(&p_mode) = args.first() {
                    if p_mode != 0 {
                        let _ = bus.write_u32(p_mode + 0, 1280); // Width
                        let _ = bus.write_u32(p_mode + 4, 720);  // Height
                        let _ = bus.write_u32(p_mode + 8, 0);    // Interlaced (0=Progressive)
                        let _ = bus.write_u32(p_mode + 12, 1);   // Widescreen (1=16:9)
                        let _ = bus.write_u32(p_mode + 16, 60);  // RefreshRate (60Hz)
                    }
                }
                0
            }

            // 0x01BD: VdGetSystemCommandBuffer
            0x01BD => 0x1000_0000,

            // 0x01BE: VdGlobalDevice
            0x01BE => 0x1001_0000,

            // 0x01C0: VdGpuClockInMHz
            0x01C0 => 500, // 500 MHz ATI Xenos

            // 0x01C1: VdHSIOCalibrationLock
            0x01C1 => 0,

            // 0x01C2: VdInitializeEngines
            0x01C2 => 0,

            // 0x01C3: VdInitializeRingBuffer
            0x01C3 => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        self.ring_buffer_ptr = ptr;
                    }
                }
                if let Some(&size) = args.get(1) {
                    if size != 0 {
                        let bytes = if size <= 32 {
                            let comp = (1u64 << size) * 4;
                            comp.max(32 * 1024) as u32
                        } else {
                            size as u32
                        };
                        self.ring_buffer_size = bytes;
                    }
                }
                println!("    -> VdInitializeRingBuffer: Address=0x{:08X}, Size=0x{:X} ({} KB)",
                    self.ring_buffer_ptr, self.ring_buffer_size, self.ring_buffer_size / 1024);
                0
            }

            // 0x01C5: VdInitializeScalerCommandBuffer
            0x01C5 => 0,

            // 0x01C6: VdIsHSIOTrainingSucceeded
            0x01C6 => 1, // TRUE

            // 0x01C7: VdPersistDisplay
            0x01C7 => 0,

            // 0x01C9: VdQueryVideoFlags
            0x01C9 => 0x0000_0001, // Widescreen 16:9

            // 0x01CA: VdQueryVideoMode(X_VIDEO_MODE*) - see kernel/video.rs
            0x01CA => {
                let video = self.video;
                video.write_video_mode(bus, args.first().copied().unwrap_or(0));
                0
            }

            // 0x01CF..0x01DC: Vd display controls
            0x01CF | 0x01D0 | 0x01D3 | 0x01D4 | 0x01DC => 0,

            // 0x01D5: VdSetGraphicsInterruptCallback (pfnCallback, pContext)
            0x01D5 => {
                if let Some(&cb) = args.first() {
                    self.graphics_interrupt_callback = cb;
                }
                if let Some(&ctx) = args.get(1) {
                    self.graphics_interrupt_context = ctx;
                }
                println!("    -> VdSetGraphicsInterruptCallback: Callback=0x{:08X}, Context=0x{:08X}",
                    self.graphics_interrupt_callback, self.graphics_interrupt_context);
                0
            }

            // 0x01D9: VdSetSystemCommandBufferGpuIdentifierAddress (pGpuIdent)
            0x01D9 => {
                if let Some(&ptr) = args.first() {
                    self.gpu_identifier_ptr = ptr;
                    if ptr != 0 {
                        let _ = bus.write_u32(ptr, 0x0000_0001);
                    }
                }
                0
            }

            // 0x025B: VdSwap (Presentation / Frame Swap)
            0x025B => {
                self.swap_count += 1;
                if let Some(&fb) = args.first() {
                    if fb != 0 {
                        self.last_swap_frontbuffer = fb;
                    }
                }
                0
            }

            // 0x0269: VdRetrainEDRAM | 0x026A: VdRetrainEDRAMWorker
            0x0269 | 0x026A => 0,

            // 0x0270: VdEnumerateVideoModes | 0x027E: VdGetDisplayDiscoveryData
            0x0270 | 0x027E => 0,

            // 0x0379: VdSetStudioRGBMode
            0x0379 => 0,

            _ => return None,
        })
    }
}
