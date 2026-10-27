//! xam.xex HLE: NetDll_*/XNet*/XHttp*: networking (offline answers today).

use super::super::dispatcher::KernelDispatcher;
use crate::kernel::objects::ObjectKind;
use crate::memory::MemoryBus;

/// XnAddrStatus (Xenia xam_net.cc).
const XNET_GET_XNADDR_STATIC: u64 = 0x0000_0004;

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

            // 0x000B: NetDll_bind (caller, s, pName, namelen)
            0x000B => {
                let s = args.get(1).copied().unwrap_or(0) as u32;
                let p_name = args.get(2).copied().unwrap_or(0);
                if p_name != 0 {
                    let port = bus.read_u16(p_name + 2).unwrap_or(0);
                    let mut ip = [0u8; 4];
                    let _ = bus.read_bytes(p_name + 4, &mut ip);
                    if self.host.net().bind(s, ip, port).is_ok() {
                        0
                    } else {
                        0xFFFF_FFFF
                    }
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x000C: NetDll_connect (caller, s, pName, namelen)
            0x000C => {
                let s = args.get(1).copied().unwrap_or(0) as u32;
                let p_name = args.get(2).copied().unwrap_or(0);
                if p_name != 0 {
                    let port = bus.read_u16(p_name + 2).unwrap_or(0);
                    let mut ip = [0u8; 4];
                    let _ = bus.read_bytes(p_name + 4, &mut ip);
                    if self.host.net().connect(s, ip, port).is_ok() {
                        0
                    } else {
                        0xFFFF_FFFF
                    }
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x0012: NetDll_recv (caller, s, buf, len, flags)
            0x0012 => {
                let s = args.get(1).copied().unwrap_or(0) as u32;
                let p_buf = args.get(2).copied().unwrap_or(0);
                let len = args.get(3).copied().unwrap_or(0) as usize;
                if p_buf != 0 && len > 0 {
                    let mut data = vec![0u8; len];
                    match self.host.net().recv(s, &mut data) {
                        Ok(n) => {
                            let _ = bus.load_bytes(p_buf, &data[..n]);
                            n as u64
                        }
                        Err(_) => 0xFFFF_FFFF,
                    }
                } else {
                    0
                }
            }

            // 0x0016: NetDll_send (caller, s, buf, len, flags)
            0x0016 => {
                let s = args.get(1).copied().unwrap_or(0) as u32;
                let p_buf = args.get(2).copied().unwrap_or(0);
                let len = args.get(3).copied().unwrap_or(0) as usize;
                if p_buf != 0 && len > 0 {
                    let mut data = vec![0u8; len];
                    let _ = bus.read_bytes(p_buf, &mut data);
                    match self.host.net().send(s, &data) {
                        Ok(n) => n as u64,
                        Err(_) => 0xFFFF_FFFF,
                    }
                } else {
                    0
                }
            }

            // NetDll_* exports 0x0005..0x0018, 0x006C..0x006F
            0x0005 | 0x0006 | 0x0007 | 0x0009 | 0x000A | 0x000D | 0x000E
            | 0x000F | 0x0010 | 0x0011 | 0x0013 | 0x0014 | 0x0017 | 0x0018
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

            // 0x0036 NetDll_XNetCreateKey(caller, pKeyId, pExchangeKey): Xenia (xam_net.cc) fills the 8-byte key id and
            // the 16-byte exchange key with 0xBE and succeeds.
            0x0036 => {
                let (key_id, exchange_key) = (args.get(1).copied().unwrap_or(0), args.get(2).copied().unwrap_or(0));
                if key_id != 0 {
                    let _ = bus.load_bytes(key_id, &[0xBE; 8]);
                }
                if exchange_key != 0 {
                    let _ = bus.load_bytes(exchange_key, &[0xBE; 16]);
                }
                0
            }

            // 0x0049 NetDll_XNetGetTitleXnAddr(caller, pXnAddr) -> XNET_GET_XNADDR_STATIC. As Xenia (xam_net.cc): XNADDR
            // { ina, inaOnline (network byte order), be16 wPortOnline, abEnet[6], abOnline[20] } with a loopback ina,
            // abEnet 0xCC (a zero MAC makes RakNet's startup fail) and the online fields zero.
            0x0049 => {
                let p_xn = args.get(1).copied().unwrap_or(0);
                if p_xn != 0 {
                    let mut xnaddr = [0u8; 36];
                    xnaddr[0..4].copy_from_slice(&[127, 0, 0, 1]);
                    xnaddr[10..16].copy_from_slice(&[0xCC; 6]);
                    let _ = bus.load_bytes(p_xn, &xnaddr);
                }
                XNET_GET_XNADDR_STATIC
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

    #[test]
    fn test_xam_net_socket_lifecycle() {
        let mut disp = KernelDispatcher::new();
        let mut bus = MemoryBus::new();

        // 0x0003: NetDll_socket (caller, af=2, ty=1, proto=6)
        let sock_handle = disp.xam_net(0x0003, &[0, 2, 1, 6], &mut bus).unwrap();
        assert_ne!(sock_handle, 0xFFFF_FFFF);

        // 0x0004: NetDll_closesocket
        let close_res = disp.xam_net(0x0004, &[0, sock_handle], &mut bus);
        assert_eq!(close_res, Some(0));
    }

    #[test]
    fn xnet_key_and_title_address_follow_xenia() {
        let mut disp = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // XNetCreateKey
        assert_eq!(disp.xam_net(0x0036, &[0, 0x1000, 0x1100], &mut bus), Some(0));
        assert_eq!(bus.read_u64(0x1000).unwrap(), 0xBEBE_BEBE_BEBE_BEBE);
        assert_eq!(bus.read_u64(0x1108).unwrap(), 0xBEBE_BEBE_BEBE_BEBE);
        assert_eq!(bus.read_u8(0x1110).unwrap(), 0);
        // XNetGetTitleXnAddr
        bus.write_u32(0x2020, 0xFFFF_FFFF).unwrap();
        assert_eq!(disp.xam_net(0x0049, &[0, 0x2000], &mut bus), Some(super::XNET_GET_XNADDR_STATIC));
        assert_eq!(bus.read_u32(0x2000).unwrap(), 0x7F00_0001);
        assert_eq!(bus.read_u32(0x2004).unwrap(), 0);
        assert_eq!(bus.read_u16(0x2008).unwrap(), 0);
        assert_eq!(bus.read_u16(0x200A).unwrap(), 0xCCCC);
        assert_eq!(bus.read_u32(0x200C).unwrap(), 0xCCCC_CCCC);
        assert_eq!(bus.read_u32(0x2020).unwrap(), 0, "abOnline cleared up to byte 36");
    }
}
