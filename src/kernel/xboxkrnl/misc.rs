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

            // 0x0021: FscSetCacheElementCount | 0x0022: FscSetMaxCacheElements
            0x0021 | 0x0022 => 0,

            // 0x0024: HalOpenCloseODDTray
            0x0024 => 0,

            // 0x0029: HalSendSMCMessage
            0x0029 => 0,

            // 0x002C: HalReturnToFirmware
            0x002C => 0,

            // 0x0158: XboxKrnlVersion
            0x0158 => 0x2011_B400, // 2.0.4532.0 (Genuine Dashboard Kernel)

            // 0x0194: XexCheckExecutablePrivilege
            0x0194 => 1, // TRUE (Privilege Granted)

            // 0x0195: XexGetModuleHandle
            0x0195 => {
                let mod_name_ptr = args.first().copied().unwrap_or(0);
                if mod_name_ptr == 0 {
                    0x9200_0000 // Self (Dashboard)
                } else {
                    0x9000_0000
                }
            }

            // 0x0196: XexGetModuleSection (hMod, name, pBase, pSize)
            0x0196 => {
                if let Some(&p_base) = args.get(2) {
                    if p_base != 0 {
                        let _ = bus.write_u32(p_base, 0x9210_5000);
                    }
                }
                if let Some(&p_sz) = args.get(3) {
                    if p_sz != 0 {
                        let _ = bus.write_u32(p_sz, 0x0058_A894);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x0197: XexGetProcedureAddress
            0x0197 => 0x9268_DD14,

            // 0x0199: XexLoadImage | 0x01A1: XexUnloadImage | 0x01A2: XexUnloadImageAndExitThread | 0x01A4: XexVerifyImageHeaders
            0x0199 | 0x01A1 | 0x01A2 | 0x01A4 => 0,

            // 0x01A5: __C_specific_handler
            0x01A5 => 0,

            // 0x01E7: XInputdWriteState | 0x01E8: XInputdNotify
            0x01E7 | 0x01E8 => 0,

            // 0x01F3..0x0200: XAudio Handlers
            0x01F3 | 0x01F4 | 0x01F5 | 0x01F7 | 0x01F8 | 0x01FA | 0x01FB | 0x01FC | 0x0200 => 0,

            // 0x021C: XUsbcamSetConfig | 0x021D: XUsbcamGetState | 0x0221: XUsbcamGetView
            0x021C | 0x021D | 0x0221 => 0,

            // 0x0224: XMACreateContext | 0x0226: XMAReleaseContext
            0x0224 => 0x0000_3000,
            0x0226 => 0,

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
