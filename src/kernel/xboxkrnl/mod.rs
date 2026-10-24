//! xboxkrnl.exe HLE exports, one file per area. Each area returns `None` for ordinals it does not handle;
//! ordinals are disjoint across areas, so the try order does not change behavior.

mod ex;
mod ke;
mod mm;
mod nt;
mod rtl;
mod xe_crypt;
mod vd;
mod misc;

use super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    pub(super) fn dispatch_xboxkrnl(
        &mut self,
        ordinal: u16,
        _func_name: &str,
        args: &[u64],
        bus: &mut MemoryBus,
    ) -> u64 {
        self.xboxkrnl_ex(ordinal, args, bus)
            .or_else(|| self.xboxkrnl_ke(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_mm(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_nt(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_rtl(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_xe_crypt(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_vd(ordinal, args, bus))
            .or_else(|| self.xboxkrnl_misc(ordinal, args, bus))
            .unwrap_or_else(|| {
                println!("    [HLE KERNEL] Unhandled xboxkrnl.exe ordinal 0x{:04X} ({})", ordinal, ordinal);
                0
            })
    }
}
