//! xam.xex HLE: NetDll_*/XNet*/XHttp*: networking (offline answers today).

use super::super::dispatcher::KernelDispatcher;
use crate::kernel::objects::ObjectKind;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_net(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x001A: NetDll_inet_addr(addr) -> the dotted IPv4 address, first byte most significant; -1 if invalid,
            // 0 for an empty string (Xenia xam_net.cc)
            0x001A => {
                let p = args.first().copied().unwrap_or(0);
                if p == 0 {
                    return Some(0xFFFF_FFFF);
                }
                let text = crate::kernel::util::read_c_string(p, bus).unwrap_or_default();
                if text.is_empty() {
                    return Some(0);
                }
                let parts: Vec<Option<u8>> = text.split('.').map(|x| x.parse().ok()).collect();
                if parts.len() == 4 && parts.iter().all(|x| x.is_some()) {
                    parts.iter().fold(0u64, |acc, b| acc << 8 | b.unwrap() as u64)
                } else {
                    0xFFFF_FFFF
                }
            }

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
                let s = self.new_handle(ObjectKind::Socket);
                s as u64
            }

            // NetDll_* exports 0x0004..0x0018, 0x006C..0x006F
            0x0004 | 0x0005 | 0x0006 | 0x0007 | 0x0009 | 0x000A | 0x000B | 0x000D | 0x000E
            | 0x000F | 0x0010 | 0x0011 | 0x0012 | 0x0013 | 0x0014 | 0x0016 | 0x0017 | 0x0018
            | 0x006C | 0x006D | 0x006E | 0x006F => 0,

            // 0x0024 NetDll_WSAStartupEx / 0x0033 NetDll_XNetStartup: success
            0x0024 | 0x0033 => 0,
            // 0x00C9 NetDll_XHttpStartup / 0x00CA NetDll_XHttpShutdown return BOOL (TRUE). The dash treats 0 as failure
            // and then reads GetLastError(), skipping the rest of its service initialisation (this left the singleton
            // at 0x92A7C23C unset and led to the call through NULL at frame 173).
            0x00C9 | 0x00CA => 1,
            // 0x004B NetDll_XNetGetEthernetLinkStatus: no cable
            0x004B => 0,

            _ => return None,
        })
    }
}
