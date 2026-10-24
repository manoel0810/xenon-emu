//! xam.xex HLE: NetDll_*/XNet*/XHttp*: networking (offline answers today).

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_net(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x0001: NetDll_WSAStartup (caller, version, pWsaData)
            0x0001 => {
                if let Some(&p_wsa) = args.get(2) {
                    if p_wsa != 0 {
                        let _ = bus.write_u16(p_wsa + 0, 0x0202); // wVersion 2.2
                        let _ = bus.write_u16(p_wsa + 2, 0x0202); // wHighVersion 2.2
                    }
                }
                0 // SUCCESS
            }

            // 0x0002: NetDll_WSACleanup
            0x0002 => 0,

            // 0x0003: NetDll_socket
            0x0003 => {
                let s = self.next_handle;
                self.next_handle += 4;
                s as u64
            }

            // 0x0004..0x0018: Network calls
            0x0004 | 0x0005 | 0x0006 | 0x0007 | 0x0009 | 0x000A | 0x000B | 0x000D | 0x000E
            | 0x000F | 0x0010 | 0x0011 | 0x0012 | 0x0013 | 0x0014 | 0x0016 | 0x0017 | 0x0018
            | 0x006C | 0x006D | 0x006E | 0x006F => 0,

            // 0x007D: XamAppAttach
            0x007D => 0,

            // 0x0024 NetDll_WSAStartupEx / 0x0033 NetDll_XNetStartup: success
            0x0024 | 0x0033 => 0,
            // 0x00C9 NetDll_XHttpStartup / 0x00CA NetDll_XHttpShutdown return BOOL (TRUE). The dash treats 0 as failure
            // and then reads GetLastError(), skipping the rest of its service initialisation (this left the singleton
            // at 0x92A7C23C unset and led to the call through NULL at frame 173).
            0x00C9 | 0x00CA => 1,
            // 0x004B NetDll_XNetGetEthernetLinkStatus: no cable
            0x004B => 0,

            // 0x028B: XNotifyGetNext (hListener, flags, pId, pParam)
            0x028B => {
                if let Some(&p_id) = args.get(2) {
                    if p_id != 0 {
                        let _ = bus.write_u32(p_id, 0); // No pending notifications
                    }
                }
                0
            }

            _ => return None,
        })
    }
}
