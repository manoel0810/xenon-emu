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

            // 0x0003: NetDll_socket (caller, af, ty, proto)
            0x0003 => {
                let af = args.get(1).copied().unwrap_or(2) as i32;
                let ty = args.get(2).copied().unwrap_or(1) as i32;
                let proto = args.get(3).copied().unwrap_or(0) as i32;
                let _ = self.host.net().socket(af, ty, proto);
                let s = self.new_handle(ObjectKind::Socket);
                s as u64
            }

            // 0x0004: NetDll_closesocket (caller, s)
            0x0004 => {
                let s = args.get(1).copied().unwrap_or(0) as u32;
                let _ = self.host.net().close_socket(s);
                0
            }

            // NetDll_* exports 0x0005..0x0018, 0x006C..0x006F
            0x0005 | 0x0006 | 0x0007 | 0x0009 | 0x000A | 0x000B | 0x000D | 0x000E
            | 0x000F | 0x0010 | 0x0011 | 0x0012 | 0x0013 | 0x0014 | 0x0016 | 0x0017 | 0x0018
            | 0x006C | 0x006D | 0x006E | 0x006F => 0,

            // 0x0024 NetDll_WSAStartupEx / 0x0033 NetDll_XNetStartup: success
            0x0024 | 0x0033 => 0,
            // 0x00C9 NetDll_XHttpStartup / 0x00CA NetDll_XHttpShutdown return BOOL (TRUE). The dash treats 0 as failure
            // and then reads GetLastError(), skipping the rest of its service initialisation (this left the singleton
            // at 0x92A7C23C unset and led to the call through NULL at frame 173).
            0x00C9 | 0x00CA => 1,
            // 0x004B NetDll_XNetGetEthernetLinkStatus: query link status from host
            0x004B => match self.host.net().link_status() {
                crate::host::traits::LinkStatus::Disconnected => 0,
                crate::host::traits::LinkStatus::Connected { .. } => 0x0000_0001 | 0x0000_0002, // XNET_ETHERNET_LINK_ACTIVE | 100MBPS
            },

            // 0x0036 NetDll_XNetGetTitleXnAddr (caller, pXnAddr) -> XNET_GET_XNADDR_ETHERNET
            0x0036 => {
                let p_xn = args.get(1).copied().unwrap_or(0);
                if p_xn != 0 {
                    match self.host.net().link_status() {
                        crate::host::traits::LinkStatus::Connected { ip, .. } => {
                            let _ = bus.load_bytes(p_xn + 0x10, &ip);
                            0x0000_0001 // XNET_GET_XNADDR_ETHERNET
                        }
                        crate::host::traits::LinkStatus::Disconnected => {
                            0x0000_0000 // None
                        }
                    }
                } else {
                    0
                }
            }

            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::dispatcher::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn test_xam_net_link_status_offline() {
        let mut disp = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // 0x004B: NetDll_XNetGetEthernetLinkStatus
        let res = disp.xam_net(0x004B, &[], &mut bus);
        assert_eq!(res, Some(0)); // Disconnected in offline mode
    }

    #[test]
    fn test_xam_net_wsa_startup() {
        let mut disp = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        let wsa_ptr = 0x4000_1000;
        let res = disp.xam_net(0x0001, &[0, 0x0202, wsa_ptr], &mut bus);
        assert_eq!(res, Some(0));
        assert_eq!(bus.read_u16(wsa_ptr).unwrap(), 0x0202);
    }
}
