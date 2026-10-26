//! xam.xex HLE: XamNotify*/XNotify*: system notifications.

use super::super::dispatcher::KernelDispatcher;
use crate::kernel::objects::ObjectTable;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_notify(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(0);
        Some(match ordinal {
            // 0x028A: XamNotifyCreateListener(mask, max_version) -> listener handle (system process: the dash)
            0x028A => self.create_notify_listener(bus, arg(0), arg(1) as u32) as u64,

            // 0x0291: XamNotifyCreateListenerInternal(mask, is_system, max_version) -> listener handle
            0x0291 => self.create_notify_listener(bus, arg(0), arg(2) as u32) as u64,

            // 0x028B: XNotifyGetNext(handle, match_id, id_ptr, param_ptr) -> TRUE if a notification was dequeued
            0x028B => {
                if arg(3) != 0 {
                    let _ = bus.write_u32(arg(3), 0);
                }
                if arg(2) == 0 {
                    return Some(0);
                }
                let _ = bus.write_u32(arg(2), 0);
                let handle = self.objects.lookup(arg(0) as u32, self.current_thread).map(|_| ObjectTable::translate(arg(0) as u32, self.current_thread));
                let Some(listener) = handle.and_then(|h| self.notifications.listeners.get_mut(&h)) else { return Some(0) };
                match listener.dequeue(bus, arg(1) as u32) {
                    Some((id, param)) => {
                        let _ = bus.write_u32(arg(2), id);
                        if arg(3) != 0 {
                            let _ = bus.write_u32(arg(3), param);
                        }
                        1
                    }
                    None => 0,
                }
            }

            // 0x028E: XNotifyBroadcast(notification, data) -> X_ERROR_SUCCESS
            0x028E => {
                self.notifications.broadcast(bus, arg(0) as u32, arg(1) as u32);
                0
            }

            // notification exports 0x028C, 0x028D, 0x0290, 0x0292, 0x02A0, 0x02A1
            0x028C | 0x028D | 0x0290 | 0x0292 | 0x02A0 | 0x02A1 => 0,

            _ => return None,
        })
    }
}
