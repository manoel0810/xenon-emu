//! xam.xex HLE: XamUser*/XamProfile*: signed-in users and profile settings.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_user(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x020A: XamUserGetXUID (userIndex, flags, pXuid)
            0x020A => {
                if let Some(&p_xuid) = args.get(2) {
                    if p_xuid != 0 {
                        let _ = bus.write_u32(p_xuid + 0, 0x0009_0000);
                        let _ = bus.write_u32(p_xuid + 4, 0x0000_0001);
                    }
                }
                0
            }

            // 0x020B: XamUserLogon
            0x020B => 0,

            // 0x020C: XamUserGetGamerTag (userIndex, pBuffer, cchBuffer)
            0x020C => {
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch >= 8 {
                        let tag = b"Xbox360\0";
                        for (i, &b) in tag.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                0
            }


            // 0x0210: XamUserGetSigninState (userIndex)
            0x0210 => self.signin_state as u64, // 1 = Signed in locally

            // 0x0211: XamUserGetIndexFromXUID (xuid, pUserIndex)
            0x0211 => {
                if let Some(&p_idx) = args.get(1) {
                    if p_idx != 0 {
                        let _ = bus.write_u32(p_idx, 0);
                    }
                }
                0
            }

            // 0x021B: XamUserGetMembershipTier
            0x021B => 6, // Gold Membership

            // 0x021C: XamUserGetUserFlags (userIndex, flags, pFlags*) - report "signed in locally, no Live"
            0x021C => {
                if let Some(&p) = args.get(3) { if p != 0 { let _ = bus.write_u32(p, 0); } }
                0
            }

            // 0x03C3: XamUserGetUserTenure (userIndex, pTenureMonths*, pAccountYears*, pLiveYears*) - brand-new local profile
            // 0x03C4: XamUserGetSubscriptionType (userIndex, pType*, pExpiry*) - none (offline)
            0x03C3 | 0x03C4 => {
                for k in 1..=3usize {
                    if let Some(&p) = args.get(k) { if p != 0 && (p & 3) == 0 { let _ = bus.write_u32(p, 0); } }
                }
                0
            }

            // 0x0219: XamUserReadProfileSettings (titleId, flags, userCount, pXuids, settingCount, pIds, pcbResults, pResults, overlapped)
            // Offline: no profile settings exist. Report an empty, well-formed XUSER_READ_PROFILE_SETTING_RESULT
            // (settings_count = 0, settings ptr = NULL) so callers do not wait on garbage.
            0x0219 => {
                let pcb = args.get(6).copied().unwrap_or(0);
                let p_res = args.get(7).copied().unwrap_or(0);
                if p_res != 0 {
                    for off in (0..0x10u64).step_by(4) { let _ = bus.write_u32(p_res + off, 0); }
                }
                if pcb != 0 { let _ = bus.write_u32(pcb, 0x10); }
                0 // ERROR_SUCCESS
            }

            // 0x0220: XamUserCheckPrivilege (userIndex, privilegeType, pGranted)
            0x0220 => {
                if let Some(&p_granted) = args.get(2) {
                    if p_granted != 0 {
                        let _ = bus.write_u32(p_granted, 1); // 1 = TRUE (Granted)
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0221: XamUserGetSigninInfo (userIndex, flags, pSigninInfo)
            0x0221 => {
                if let Some(&p_info) = args.get(2) {
                    if p_info != 0 {
                        let _ = bus.write_u32(p_info + 0, 0x0009_0000); // xuid high
                        let _ = bus.write_u32(p_info + 4, 0x0000_0001); // xuid low
                        let _ = bus.write_u32(p_info + 8, 0);           // dwInfoFlags
                        let _ = bus.write_u32(p_info + 12, self.signin_state as u32); // 1 = Signed in locally
                        let _ = bus.write_u32(p_info + 16, 0);          // guest number
                        let _ = bus.write_u32(p_info + 20, 0);          // sponsor user index
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0222: XamUserGetName
            0x0222 => {
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch >= 8 {
                        let tag = b"Xbox360\0";
                        for (i, &b) in tag.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                0
            }

            // 0x0227: XamUserCreateAchievementEnumerator
            0x0227 => {
                if let Some(&p_h) = args.get(4) {
                    if p_h != 0 {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        let _ = bus.write_u32(p_h, h);
                    }
                }
                0
            }

            // 0x0230: XamProfileCreate | 0x0234: XamProfileGetCreationStatus
            0x0230 | 0x0234 => 0,

            _ => return None,
        })
    }
}
