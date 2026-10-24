//! xam.xex HLE exports, one file per area. Each area returns `None` for ordinals it does not handle;
//! ordinals are disjoint across areas, so the try order does not change behavior.

mod net;
mod user;
mod content;
mod notify;
mod ui;
mod input;
mod app;
mod misc;

use super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    pub(super) fn dispatch_xam(
        &mut self,
        ordinal: u16,
        _func_name: &str,
        args: &[u64],
        bus: &mut MemoryBus,
    ) -> u64 {
        self.xam_net(ordinal, args, bus)
            .or_else(|| self.xam_user(ordinal, args, bus))
            .or_else(|| self.xam_content(ordinal, args, bus))
            .or_else(|| self.xam_notify(ordinal, args, bus))
            .or_else(|| self.xam_ui(ordinal, args, bus))
            .or_else(|| self.xam_input(ordinal, args, bus))
            .or_else(|| self.xam_app(ordinal, args, bus))
            .or_else(|| self.xam_misc(ordinal, args, bus))
            .unwrap_or_else(|| {
                println!("    [HLE XAM] Unhandled xam.xex ordinal 0x{:04X} ({})", ordinal, ordinal);
                0
            })
    }
}
