//! xam.xex HLE: XamUser*/XamProfile*: signed-in users and profile settings.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

// xbox.h
const X_ERROR_INVALID_PARAMETER: u64 = 0x57;
const X_ERROR_NOT_LOGGED_ON: u64 = 0x4DD;
const X_ERROR_NO_SUCH_USER: u64 = 0x525;
const X_E_INVALIDARG: u64 = 0x8007_0057;
const X_E_NO_SUCH_USER: u64 = 0x8007_0525;

impl KernelDispatcher {
    /// Checks if a user is signed in according to host profile.
    fn user_signed_in(&self, user_index: u32) -> bool {
        if user_index >= 4 {
            false
        } else if let Some(u) = self.host.profile().get_user(user_index) {
            u.signin_state != 0
        } else {
            false
        }
    }

    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_user(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x02AE: XamUserIsUnsafeProgrammingAllowed(user_index, pc_check, result_ptr) -> *result = 1 (Xenia stub)
            0x02AE => {
                let (user_index, result_ptr) = (args.first().copied().unwrap_or(0) as u32, args.get(2).copied().unwrap_or(0));
                if result_ptr == 0 || (user_index != 0xFF && user_index >= 4) {
                    return Some(X_ERROR_INVALID_PARAMETER);
                }
                let _ = bus.write_u32(result_ptr, 1);
                0
            }

            // 0x020A: XamUserGetXUID (userIndex, flags, pXuid)
            0x020A => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                let xuid = if let Some(u) = self.host.profile().get_user(user_index) {
                    u.xuid
                } else if user_index == 0xFF {
                    self.host.profile().signed_in_users().first().map(|u| u.xuid).unwrap_or(0x0009_0000_0000_0001)
                } else {
                    return Some(X_ERROR_NO_SUCH_USER);
                };
                if let Some(&p_xuid) = args.get(2) {
                    if p_xuid != 0 {
                        let _ = bus.write_u64(p_xuid, xuid);
                    }
                }
                0
            }

            // 0x020B: XamUserLogon
            0x020B => 0,

            // 0x020C: XamUserGetGamerTag (userIndex, pBuffer, cchBuffer)
            0x020C => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                let gamertag = if let Some(u) = self.host.profile().get_user(user_index) {
                    u.gamertag
                } else if user_index == 0xFF {
                    self.host.profile().signed_in_users().first().map(|u| u.gamertag.clone()).unwrap_or_else(|| "Xbox Player".to_string())
                } else {
                    return Some(X_ERROR_NO_SUCH_USER);
                };
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch > 0 {
                        let bytes = gamertag.as_bytes();
                        let len = bytes.len().min(cch as usize - 1);
                        for i in 0..len {
                            let _ = bus.write_u8(p_buf + i as u64, bytes[i]);
                        }
                        let _ = bus.write_u8(p_buf + len as u64, 0);
                    }
                }
                0
            }

            // 0x0210: XamUserGetSigninState(user_index) -> the user's sign-in state, 0 for an empty slot or an index
            // >= 4 such as XUSER_INDEX_ANY (Xenia xam_user.cc); 1 = signed in locally
            0x0210 => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                if user_index == 0xFF {
                    self.host.profile().signed_in_users().first().map(|u| u.signin_state as u64).unwrap_or(0)
                } else if let Some(u) = self.host.profile().get_user(user_index) {
                    u.signin_state as u64
                } else {
                    0
                }
            }

            // 0x0211: XamUserGetIndexFromXUID (xuid, pUserIndex)
            0x0211 => {
                let xuid = args.first().copied().unwrap_or(0);
                let mut found = None;
                for user in self.host.profile().signed_in_users() {
                    if user.xuid == xuid {
                        found = Some(user.slot);
                        break;
                    }
                }
                if let Some(&p_idx) = args.get(1) {
                    if p_idx != 0 {
                        let _ = bus.write_u32(p_idx, found.unwrap_or(0));
                    }
                }
                match found {
                    Some(_) => 0,
                    None => X_ERROR_NO_SUCH_USER,
                }
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

            // 0x0212: XamUserCheckPrivilege(user_index, mask, out_value) (Xenia xam_user.cc): users signed in
            // locally, not to Live, get *out = 0 and X_ERROR_NOT_LOGGED_ON
            0x0212 => {
                let (user_index, out_ptr) = (args.first().copied().unwrap_or(0) as u32, args.get(2).copied().unwrap_or(0));
                let write_out = |bus: &mut MemoryBus| if out_ptr != 0 { let _ = bus.write_u32(out_ptr, 0); };
                if user_index == 0xFF {
                    // XUserIndexAny: the first signed-in user's result
                    write_out(bus);
                    if self.user_signed_in(0) { X_ERROR_NOT_LOGGED_ON } else { X_ERROR_NO_SUCH_USER }
                } else if user_index >= 4 {
                    X_ERROR_INVALID_PARAMETER
                } else if !self.user_signed_in(user_index) {
                    X_ERROR_NO_SUCH_USER
                } else {
                    write_out(bus);
                    X_ERROR_NOT_LOGGED_ON
                }
            }

            // 0x0221: XamUserIsOnlineEnabled(user_index) -> 0: the local profile is not Live-enabled
            0x0221 => 0,

            // 0x0227: XamUserGetSigninInfo(user_index, flags, info) -> X_HRESULT (Xenia xam_user.cc): the
            // XUSER_SIGNIN_INFO (0x28 bytes) is zeroed, then filled for a signed-in user
            0x0227 => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                let p_info = args.get(2).copied().unwrap_or(0);
                if p_info == 0 {
                    return Some(X_E_INVALIDARG);
                }
                for off in (0..0x28u64).step_by(4) {
                    let _ = bus.write_u32(p_info + off, 0);
                }
                if user_index >= 4 && user_index != 0xFF {
                    X_E_INVALIDARG
                } else if !self.user_signed_in(user_index) {
                    X_E_NO_SUCH_USER
                } else {
                    let user = if user_index == 0xFF {
                        self.host.profile().signed_in_users().into_iter().next()
                    } else {
                        self.host.profile().get_user(user_index)
                    };
                    if let Some(user) = user {
                        let _ = bus.write_u64(p_info, user.xuid);
                        let _ = bus.write_u32(p_info + 12, user.signin_state);
                        let b = user.gamertag.as_bytes();
                        let len = b.len().min(15);
                        for i in 0..len {
                            let _ = bus.write_u8(p_info + 24 + i as u64, b[i]);
                        }
                        0 // X_E_SUCCESS
                    } else {
                        X_E_NO_SUCH_USER
                    }
                }
            }

            // 0x020E: XamUserGetName(user_index, buffer, buffer_len)
            0x020E => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                let gamertag = if let Some(u) = self.host.profile().get_user(user_index) {
                    u.gamertag
                } else if user_index == 0xFF {
                    self.host.profile().signed_in_users().first().map(|u| u.gamertag.clone()).unwrap_or_else(|| "Xbox Player".to_string())
                } else {
                    return Some(X_ERROR_NO_SUCH_USER);
                };
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch > 0 {
                        let bytes = gamertag.as_bytes();
                        let len = bytes.len().min(cch as usize - 1);
                        for i in 0..len {
                            let _ = bus.write_u8(p_buf + i as u64, bytes[i]);
                        }
                        let _ = bus.write_u8(p_buf + len as u64, 0);
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

#[cfg(test)]
mod tests {
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn user_ordinals_follow_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // XamUserGetSigninInfo is 0x0227 (0x0221 is XamUserIsOnlineEnabled)
        bus.write_u32(0x4000_0024, 0xEEEE_EEEE).unwrap();
        assert_eq!(k.dispatch("xam.xex", 0x0227, &[0, 0, 0x4000_0000], &mut bus), 0);
        assert_eq!((bus.read_u32(0x4000_0004).unwrap(), bus.read_u32(0x4000_000C).unwrap(), bus.read_u32(0x4000_0024).unwrap()), (1, 1, 0));
        assert_eq!(k.dispatch("xam.xex", 0x0227, &[4, 0, 0x4000_0000], &mut bus), 0x8007_0057);
        assert_eq!(k.dispatch("xam.xex", 0x0221, &[0, 0, 0x4000_0100], &mut bus), 0);
        assert_eq!(bus.read_u32(0x4000_0100).unwrap(), 0, "IsOnlineEnabled writes nothing");
        // XamUserCheckPrivilege (0x0212): signed in locally, not to Live -> NOT_LOGGED_ON and *out = 0
        bus.write_u32(0x4000_0200, 7).unwrap();
        assert_eq!(k.dispatch("xam.xex", 0x0212, &[0, 0x10, 0x4000_0200], &mut bus), 0x4DD);
        assert_eq!(bus.read_u32(0x4000_0200).unwrap(), 0);
        // XamUserFlushLogonQueue (0x0220) is not implemented: it must not write through its arguments
        bus.write_u32(0x4000_0300, 7).unwrap();
        k.dispatch("xam.xex", 0x0220, &[0, 0, 0x4000_0300], &mut bus);
        assert_eq!(bus.read_u32(0x4000_0300).unwrap(), 7);
    }
}
