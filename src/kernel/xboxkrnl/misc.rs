//! xboxkrnl.exe HLE: Everything else exported by xboxkrnl.exe (Dbg, Hal, Xex, Xbox*, ...).

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

const X_E_INVALIDARG: u64 = 0x8007_0057;
const X_E_FAIL: u64 = 0x8000_4005;
/// XAudio driver handles (Xenia xboxkrnl_audio.cc): 0x41550000 | client index.
const AUDIO_DRIVER_HANDLE: u32 = 0x4155_0000;
/// apu/audio_driver.h kFrameChannelsDefault, kChannelSamplesDefault.
const AUDIO_CHANNELS: u64 = 6;
const AUDIO_CHANNEL_SAMPLES: u64 = 256;
/// XCONFIG_USER_AUDIO_FLAGS as XGetAudioFlags reports it (Dolby Digital | Dolby Pro Logic).
const XCONFIG_USER_AUDIO_FLAGS: u32 = 0x0001_0001;

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

            // 0x0197: XexGetProcedureAddress(hmodule, ordinal_or_name, out_ptr) - see kernel/modules.rs
            0x0197 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                self.modules.xex_get_procedure_address(bus, arg(0), arg(1), arg(2))
            }

            // 0x0199: XexLoadImage(name, flags, min_version, hmodule_ptr) - see kernel/user_module.rs
            0x0199 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                self.xex_load_image(bus, arg(0), arg(3))
            }

            // 0x01A1: XexUnloadImage(hmodule) - see kernel/user_module.rs
            0x01A1 => self.xex_unload_image(bus, args.first().copied().unwrap_or(0)),

            // 0x01A2: XexUnloadImageAndExitThread | 0x01A4: XexVerifyImageHeaders
            0x01A2 | 0x01A4 => 0,

            // 0x01A5: __C_specific_handler
            0x01A5 => 0,

            // 0x01E7: XInputdWriteState | 0x01E8: XInputdNotify
            0x01E7 | 0x01E8 => 0,

            // 0x01F2: XAudioRenderDriverInitialize | 0x01F6: XAudioRenderDriverLock | 0x01FA: XAudioBeginDigitalBypassMode |
            // 0x01FB: XAudioEndDigitalBypassMode | 0x01FC: XAudioSubmitDigitalPacket | 0x0200: XAudioSetSpeakerConfig |
            // 0x0276: XAudioOverrideSpeakerConfig: not exported by Xenia's xboxkrnl_audio.cc; success
            0x01F2 | 0x01F6 | 0x01FA | 0x01FB | 0x01FC | 0x0200 | 0x0276 => 0,

            // 0x01F3: XAudioRegisterRenderDriverClient(callback_ptr -> {callback, argument}, driver_ptr): Xenia
            // registers a client with the audio system and returns the driver handle 0x41550000 | index.
            0x01F3 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let callback = if arg(0) != 0 { bus.read_u32(arg(0)).unwrap_or(0) } else { 0 };
                if callback == 0 {
                    return Some(X_E_INVALIDARG);
                }
                let argument = bus.read_u32(arg(0) + 4).unwrap_or(0);
                match self.audio_clients.iter().position(Option::is_none) {
                    Some(index) => {
                        self.audio_clients[index] = Some((callback, argument));
                        let _ = bus.write_u32(arg(1), AUDIO_DRIVER_HANDLE | index as u32);
                        0
                    }
                    None => X_E_FAIL,
                }
            }

            // 0x01F4: XAudioUnregisterRenderDriverClient(driver)
            0x01F4 => {
                let index = (args.first().copied().unwrap_or(0) & 0xFFFF) as usize;
                if let Some(client) = self.audio_clients.get_mut(index) {
                    *client = None;
                }
                0
            }

            // 0x01F5: XAudioSubmitRenderDriverFrame(driver, samples): one frame of 6 channels x 256 big-endian floats,
            // channel after channel (apu/audio_driver.h), handed to the host interleaved as 16-bit PCM.
            0x01F5 => {
                let samples = args.get(1).copied().unwrap_or(0);
                if samples != 0 {
                    let read = |c: u64, i: u64| f32::from_bits(bus.read_u32(samples + 4 * (c * AUDIO_CHANNEL_SAMPLES + i)).unwrap_or(0));
                    let pcm: Vec<i16> = (0..AUDIO_CHANNEL_SAMPLES)
                        .flat_map(|i| (0..AUDIO_CHANNELS).map(move |c| (c, i)))
                        .map(|(c, i)| (read(c, i).clamp(-1.0, 1.0) * 32767.0).round() as i16)
                        .collect();
                    self.host.audio().submit_pcm(&pcm);
                }
                0
            }

            // 0x01F7: XAudioGetVoiceCategoryVolumeChangeMask(driver, mask_ptr): no change (Xenia writes 0)
            0x01F7 => {
                let _ = bus.write_u32(args.get(1).copied().unwrap_or(0), 0);
                0
            }

            // 0x01F8: XAudioGetVoiceCategoryVolume(category, volume_ptr): full volume (Xenia writes 1.0f)
            0x01F8 => {
                let _ = bus.write_u32(args.get(1).copied().unwrap_or(0), 1f32.to_bits());
                0
            }

            // 0x034D: XAudioEnableDucker
            0x034D => 0,

            // 0x01FF: XAudioGetSpeakerConfig(config_ptr): XCONFIG_USER_AUDIO_FLAGS, the same value XGetAudioFlags
            // (xam 0x03CA) reports
            0x01FF => {
                let _ = bus.write_u32(args.first().copied().unwrap_or(0), XCONFIG_USER_AUDIO_FLAGS);
                0
            }

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
    use super::*;

    #[test]
    fn xaudio_clients_and_frames_follow_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // callback {0x82001000, 0x1234}
        bus.write_u32(0x1000, 0x8200_1000).unwrap();
        bus.write_u32(0x1004, 0x1234).unwrap();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F3, &[0x1000, 0x2000], &mut bus), 0);
        assert_eq!(bus.read_u32(0x2000).unwrap(), 0x4155_0000);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F3, &[0x1000, 0x2000], &mut bus), 0);
        assert_eq!(bus.read_u32(0x2000).unwrap(), 0x4155_0001);
        assert_eq!(k.audio_clients[1], Some((0x8200_1000, 0x1234)));
        // a null callback is rejected
        bus.write_u32(0x1100, 0).unwrap();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F3, &[0x1100, 0x2000], &mut bus), X_E_INVALIDARG);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F4, &[0x4155_0000], &mut bus), 0);
        assert_eq!(k.audio_clients[0], None);

        // frame: channel c, sample i = (c + 1) / 8 for i = 0, -1.5 (clamped) for i = 1
        let frame = 0x3000;
        for c in 0..6u64 {
            bus.write_u32(frame + 4 * (c * 256), ((c + 1) as f32 / 8.0).to_bits()).unwrap();
            bus.write_u32(frame + 4 * (c * 256 + 1), (-1.5f32).to_bits()).unwrap();
        }
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F5, &[0x4155_0001, frame], &mut bus), 0);
        let pcm = k.host.audio().drain_pcm();
        assert_eq!(pcm.len(), 6 * 256);
        assert_eq!(pcm[..6], [4096, 8192, 12288, 16384, 20479, 24575]);
        assert_eq!(pcm[6..12], [-32767; 6]);

        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01FF, &[0x4000], &mut bus), 0);
        assert_eq!(bus.read_u32(0x4000).unwrap(), XCONFIG_USER_AUDIO_FLAGS);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01F8, &[0, 0x4004], &mut bus), 0);
        assert_eq!(bus.read_u32(0x4004).unwrap(), 1f32.to_bits());
    }
}
