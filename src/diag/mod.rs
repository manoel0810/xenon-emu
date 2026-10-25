//! Diagnostic configuration: every `XENON_*` environment variable is read here, once, into `DiagConfig`.
//! Flags are "set" when the variable exists (any value), matching `std::env::var(..).is_ok()`.

use std::sync::OnceLock;

pub mod hooks;
pub mod profile;
pub mod trace;
pub mod watchdog;

#[derive(Debug, Clone, Default)]
pub struct DiagConfig {
    // ---- Run control / presentation ----
    /// XENON_HEADLESS: no OS window (output via BMP / XENON_MAX_FRAMES).
    pub headless: bool,
    /// XENON_MAX_FRAMES=<n>: stop the frame loop cleanly after n frames.
    pub max_frames: Option<u64>,
    /// XENON_SKIP_BOOTANIM: show the dashboard immediately.
    pub skip_bootanim: bool,
    /// XENON_FB_SOURCE=reference: start on the real-hardware framebuffer dump instead of guest VRAM.
    pub fb_source_reference: bool,

    // ---- Threads / traps ----
    /// XENON_TRAP_TRACE: keep a ring of the last executed instructions and dump it on trap.
    pub trap_trace: bool,
    /// XENON_TRAP_FULL: full trap dump.
    pub trap_full: bool,
    /// XENON_NO_RESTART: stop trapped threads instead of restarting them.
    pub no_restart: bool,
    /// XENON_RESTART_MAIN: allow restarting the Metro primary thread (off by default, see HANDOFF.md section 10).
    pub restart_main: bool,
    /// XENON_DUMP_RING_FRAME=<n>: dump the instruction ring of the primary thread at frame n.
    pub dump_ring_frame: Option<u64>,
    /// XENON_DUMP_RING_N=<n>: instructions printed per ring dump (default 400).
    pub dump_ring_n: usize,
    /// XENON_WATCHDOG: exit(86) after 4s without progress.
    pub watchdog: bool,
    /// XENON_HEARTBEAT: periodic progress lines.
    pub heartbeat: bool,

    // ---- Watchpoints ----
    /// XENON_WATCH=0xADDR: report writes to this guest address.
    pub watch_addr: Option<u64>,
    /// XENON_WATCH is present (even if not a valid address): verbose restart messages.
    pub watch_set: bool,
    /// XENON_WATCH_MEM: dump memory around the watched address before each write.
    pub watch_mem: bool,
    /// XENON_WATCH_DUMP_HIT=<n>: dump the instruction ring after the n-th watchpoint hit (needs XENON_TRAP_TRACE).
    pub watch_dump_hit: Option<u64>,

    // ---- PC hooks / profiling ----
    /// XENON_PC_HOOK=0xA,0xB: dump registers when the primary thread reaches these PCs.
    pub pc_hooks: Vec<u64>,
    /// XENON_PC_HOOK_ALL: apply PC hooks to every thread.
    pub pc_hook_all: bool,
    /// XENON_PC_HOOK_MIN_FRAME=<n> (default 0).
    pub pc_hook_min_frame: u64,
    /// XENON_PC_HOOK_LIMIT=<n> (default 40).
    pub pc_hook_limit: u64,
    /// XENON_PC_HOOK_MEM="<reg>:<hex off>:<words>".
    pub pc_hook_mem: Option<String>,
    /// XENON_PC_HOOK_LIST="<reg>".
    pub pc_hook_list: Option<String>,
    /// XENON_PC_HOOK_STRREGS="4,3".
    pub pc_hook_strregs: Option<String>,
    /// XENON_PC_HOOK_R3STR.
    pub pc_hook_r3str: bool,
    /// XENON_PC_PROFILE: print hot PCs and syscalls at exit.
    pub pc_profile: bool,

    // ---- Logs ----
    /// XENON_SYSCALL_LOG: log syscalls from the frame loop.
    pub syscall_log: bool,
    /// XENON_SYSCALL_SUMMARY: aggregated syscall table at exit.
    pub syscall_summary: bool,
    /// XENON_LOG_SYSCALLS: log every HLE dispatch inside the kernel.
    pub log_syscalls: bool,
    /// XENON_GPU_MMIO_LOG: log GPU MMIO accesses.
    pub gpu_mmio_log: bool,
    /// XENON_PM4_LOG: print every PM4 packet the Command Processor executes.
    pub pm4_log: bool,
    /// XENON_DRAW_LOG: print every draw packet with its render-target/viewport/shader/resolve state (gpu/draw_log.rs).
    pub draw_log: bool,
    /// XENON_MMIO_WRITE_LOG: print every CPU store into the GPU register window (register, value, pc, thread).
    pub mmio_write_log: bool,
    /// XENON_LIST_IMPORTS: print the dash's imported functions (library, ordinal, export name) at boot.
    pub list_imports: bool,
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok()
}

fn string(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    string(name).and_then(|s| s.parse().ok())
}

fn hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

impl DiagConfig {
    pub fn from_env() -> Self {
        Self {
            headless: flag("XENON_HEADLESS"),
            max_frames: parse("XENON_MAX_FRAMES"),
            skip_bootanim: flag("XENON_SKIP_BOOTANIM"),
            fb_source_reference: string("XENON_FB_SOURCE").map(|v| v.eq_ignore_ascii_case("reference")).unwrap_or(false),

            trap_trace: flag("XENON_TRAP_TRACE"),
            trap_full: flag("XENON_TRAP_FULL"),
            no_restart: flag("XENON_NO_RESTART"),
            restart_main: flag("XENON_RESTART_MAIN"),
            dump_ring_frame: parse("XENON_DUMP_RING_FRAME"),
            dump_ring_n: parse("XENON_DUMP_RING_N").unwrap_or(400),
            watchdog: flag("XENON_WATCHDOG"),
            heartbeat: flag("XENON_HEARTBEAT"),

            watch_addr: string("XENON_WATCH").and_then(|s| hex(&s)),
            watch_set: flag("XENON_WATCH"),
            watch_mem: flag("XENON_WATCH_MEM"),
            watch_dump_hit: parse("XENON_WATCH_DUMP_HIT"),

            pc_hooks: string("XENON_PC_HOOK").map(|s| s.split(',').filter_map(|p| hex(p.trim())).collect()).unwrap_or_default(),
            pc_hook_all: flag("XENON_PC_HOOK_ALL"),
            pc_hook_min_frame: parse("XENON_PC_HOOK_MIN_FRAME").unwrap_or(0),
            pc_hook_limit: parse("XENON_PC_HOOK_LIMIT").unwrap_or(40),
            pc_hook_mem: string("XENON_PC_HOOK_MEM"),
            pc_hook_list: string("XENON_PC_HOOK_LIST"),
            pc_hook_strregs: string("XENON_PC_HOOK_STRREGS"),
            pc_hook_r3str: flag("XENON_PC_HOOK_R3STR"),
            pc_profile: flag("XENON_PC_PROFILE"),

            syscall_log: flag("XENON_SYSCALL_LOG"),
            syscall_summary: flag("XENON_SYSCALL_SUMMARY"),
            log_syscalls: flag("XENON_LOG_SYSCALLS"),
            gpu_mmio_log: flag("XENON_GPU_MMIO_LOG"),
            pm4_log: flag("XENON_PM4_LOG"),
            draw_log: flag("XENON_DRAW_LOG"),
            mmio_write_log: flag("XENON_MMIO_WRITE_LOG"),
            list_imports: flag("XENON_LIST_IMPORTS"),
        }
    }
}

/// Process-wide configuration, read from the environment on first use.
pub fn config() -> &'static DiagConfig {
    static CONFIG: OnceLock<DiagConfig> = OnceLock::new();
    CONFIG.get_or_init(DiagConfig::from_env)
}
