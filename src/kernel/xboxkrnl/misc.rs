//! xboxkrnl.exe HLE: Everything else exported by xboxkrnl.exe (Dbg, Hal, Xex, Xbox*, ...).

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_misc(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x0001: DbgBreakPoint
            0x0001 => 0,

            // 0x0003: DbgPrint
            0x0003 => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        let mut msg = String::new();
                        for i in 0..256 {
                            if let Ok(b) = bus.read_u8(ptr + i) {
                                if b == 0 {
                                    break;
                                }
                                msg.push(b as char);
                            }
                        }
                        if !msg.is_empty() {
                            println!("    >>> [GUEST DbgPrint] {}", msg.trim_end());
                        }
                    }
                }
                0
            }

            // 0x0021: FscSetCacheElementCount
            0x0021 => 0,

            // 0x0022: HalGetCurrentAVPack() -> AV pack (Xenia cvar avpack, default 8)
            0x0022 => 8,

            // 0x0024: HalOpenCloseODDTray
            0x0024 => 0,

            // 0x0029: HalSendSMCMessage
            0x0029 => 0,

            // 0x0028: HalReturnToFirmware(routine) - does not return: Xenia exits the emulator; here emulation stops
            // like XamLoaderTerminateTitle
            0x0028 => {
                println!("    -> HalReturnToFirmware({}): title requested a firmware reentry; stopping emulation", args.first().copied().unwrap_or(0));
                self.title_terminated = true;
                0
            }

            // 0x0158: XboxKrnlVersion
            0x0158 => 0x2011_B400, // 2.0.4532.0 (Genuine Dashboard Kernel)

            // 0x0194: XexCheckExecutablePrivilege
            0x0194 => 1, // TRUE (Privilege Granted)

            // 0x0195: XexGetModuleHandle(module_name, hmodule_ptr) - see kernel/modules.rs
            0x0195 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                self.modules.xex_get_module_handle(bus, arg(0), arg(1))
            }

            // 0x0196: XexGetModuleSection(hmodule, name, data_ptr, size_ptr) - see kernel/modules.rs
            0x0196 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                self.modules.xex_get_module_section(bus, arg(0), arg(1), arg(2), arg(3))
            }

            // 0x0197: XexGetProcedureAddress
            0x0197 => 0x9268_DD14,

            // 0x0199: XexLoadImage | 0x01A1: XexUnloadImage | 0x01A2: XexUnloadImageAndExitThread | 0x01A4: XexVerifyImageHeaders
            0x0199 | 0x01A1 | 0x01A2 | 0x01A4 => 0,

            // 0x01A5: __C_specific_handler
            0x01A5 => 0,

            // 0x01E7: XInputdWriteState | 0x01E8: XInputdNotify
            0x01E7 | 0x01E8 => 0,

            // 0x01F2: XAudioRenderDriverInitialize
            0x01F2 => 0,

            // 0x01F3: XAudioRegisterRenderDriverClient
            0x01F3 => {
                let p_handle = args.get(2).copied().unwrap_or(0);
                if p_handle != 0 {
                    let _ = bus.write_u32(p_handle, 1);
                }
                0
            }

            // 0x01F4: XAudioUnregisterRenderDriverClient
            0x01F4 => 0,

            // 0x01F5: XAudioSubmitRenderDriverFrame
            0x01F5 => {
                let p_frame = args.get(1).copied().unwrap_or(0);
                if p_frame != 0 {
                    let mut pcm = Vec::with_capacity(128);
                    for i in 0..128 {
                        if let Ok(sample) = bus.read_u16(p_frame + i * 2) {
                            pcm.push(sample as i16);
                        }
                    }
                    if !pcm.is_empty() {
                        self.host.audio().submit_pcm(&pcm);
                    }
                }
                0
            }

            // 0x01F6: XAudioRenderDriverLock
            0x01F6 => 0,

            // 0x01F7: XAudioGetVoiceCategoryVolumeChangeMask | 0x01F8: XAudioGetVoiceCategoryVolume | 0x01FA: XAudioBeginDigitalBypassMode | 0x01FB: XAudioEndDigitalBypassMode | 0x01FC: XAudioSubmitDigitalPacket
            0x01F7 | 0x01F8 | 0x01FA | 0x01FB | 0x01FC => 0,

            // 0x01FF: XAudioGetSpeakerConfig
            0x01FF => {
                let p_config = args.first().copied().unwrap_or(0);
                if p_config != 0 {
                    let _ = bus.write_u32(p_config, 0x0001_0001); // XAUDIOSPEAKERCONFIG_STEREO
                }
                0
            }

            // 0x0200: XAudioSetSpeakerConfig
            0x0200 => 0,

            // 0x0276: XAudioOverrideSpeakerConfig
            0x0276 => 0,

            // 0x021C: XUsbcamSetConfig | 0x021D: XUsbcamGetState | 0x0221: XUsbcamGetView
            0x021C | 0x021D | 0x0221 => 0,

            // 0x0224: XMACreateContext | 0x0226: XMAReleaseContext
            0x0224 => 0x0000_3000,
            0x0226 => 0,

            // 0x022E: XMASetInputBuffer0Valid | 0x022F: XMAIsInputBuffer0Valid | 0x0230: XMASetInputBuffer1Valid | 0x0231: XMAIsInputBuffer1Valid
            0x022E | 0x022F | 0x0230 | 0x0231 => 0,

            // 0x0232: XMASetInputBuffer0 | 0x0233: XMASetInputBuffer1 | 0x0237: XMASetInputBufferReadOffset | 0x0238: XMAGetInputBufferReadOffset
            0x0232 | 0x0233 | 0x0237 | 0x0238 => 0,

            // 0x02BD: HalRegisterXamPowerDownCallback
            0x02BD => 0,

            // 0x02E1: RmcDeviceRequest
            0x02E1 => 0,

            // 0x0334: EtxProducerRegister
            0x0334 => 0,

            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::dispatcher::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn test_xaudio_speaker_config() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();

        let cfg_ptr = 0x4000_1000;
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01FF, &[cfg_ptr], &mut bus), 0);
        assert_eq!(bus.read_u32(cfg_ptr).unwrap(), 0x0001_0001); // Stereo
    }

    #[test]
    fn test_xaudio_submit_frame_and_drain() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();

        // Register client
        let h_ptr = 0x4000_2000;
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F3, &[0, 0, h_ptr], &mut bus), 0);
        assert_eq!(bus.read_u32(h_ptr).unwrap(), 1);

        // Prepare 16 PCM stereo samples (32 i16 words)
        let frame_ptr = 0x4000_2100;
        for i in 0..32 {
            bus.write_u16(frame_ptr + i * 2, (i * 100) as u16).unwrap();
        }

        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F5, &[1, frame_ptr], &mut bus), 0);

        let drained = k.host.audio().drain_pcm();
        assert!(!drained.is_empty());
        assert_eq!(drained[0], 0);
        assert_eq!(drained[1], 100);
    }
}
