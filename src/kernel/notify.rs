// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xnotifylistener.h/.cc     XNotificationKey, Enqueue/DequeueNotification (manual-reset event)
//   src/xenia/kernel/kernel_state.cc           RegisterNotifyListener (startup notifications), BroadcastNotification
//   src/xenia/kernel/xam/xam_notify.cc         XamNotifyCreateListener(Internal), XNotifyGetNext, XNotifyBroadcast
//   src/xenia/xbox.h                           kXNotify* masks and kXNotification* ids
//   src/xenia/apu/audio_media_player.h         IsTitleInPlaybackControl defaults (game client and controller)
//
//! XAM notification listeners. Each listener is a kernel object whose wait handle is a manual-reset event (here a
//! guest KEVENT, so waits on the listener handle block in the scheduler) that is signaled while notifications are
//! queued.

use std::collections::{HashMap, VecDeque};

use super::guest_mem::GuestMem;
use super::objects::{reset_event, set_event};

pub const NOTIFY_SYSTEM: u64 = 0x01;
pub const NOTIFY_LIVE: u64 = 0x02;
pub const NOTIFY_XMP: u64 = 0x20;

pub const SYSTEM_UI: u32 = 0x0000_0009;
pub const SYSTEM_SIGN_IN_CHANGED: u32 = 0x0000_000A;
pub const SYSTEM_STORAGE_DEVICES_CHANGED: u32 = 0x0000_000B;
pub const SYSTEM_DASH_CONTEXT_CHANGED: u32 = 0x8000_000C;
pub const LIVE_CONNECTION_CHANGED: u32 = 0x0200_0001;
pub const LIVE_LINK_STATE_CHANGED: u32 = 0x0200_0003;
pub const XMP_PLAYBACK_CONTROLLER_CHANGED: u32 = 0x0A00_0003;
/// X_ONLINE_S_LOGON_DISCONNECTED
const LOGON_DISCONNECTED: u32 = 0x0015_10F1;

/// XNotificationKey: (mask index, version) of a notification id.
fn key(id: u32) -> (u32, u32) {
    ((id >> 25) & 0x3F, (id >> 16) & 0x1FF)
}

#[derive(Debug, Clone)]
pub struct Listener {
    pub mask: u64,
    pub max_version: u32,
    /// The listener's wait handle: a notification (manual-reset) KEVENT in guest memory.
    pub event: u32,
    pub queue: VecDeque<(u32, u32)>,
}

impl Listener {
    /// XNotifyListener::EnqueueNotification: ignored unless the mask matches and the version is supported.
    pub fn enqueue(&mut self, mem: &mut dyn GuestMem, id: u32, data: u32) {
        let (mask_index, version) = key(id);
        if self.mask & (1u64 << mask_index) == 0 || version > self.max_version {
            return;
        }
        self.queue.push_back((id, data));
        set_event(mem, self.event as u64);
    }

    /// XNotifyListener::DequeueNotification, the next one or the first with `match_id`.
    pub fn dequeue(&mut self, mem: &mut dyn GuestMem, match_id: u32) -> Option<(u32, u32)> {
        let i = if match_id != 0 { self.queue.iter().position(|&(id, _)| id == match_id)? } else if self.queue.is_empty() { return None } else { 0 };
        let entry = self.queue.remove(i);
        if self.queue.is_empty() {
            reset_event(mem, self.event as u64);
        }
        entry
    }
}

#[derive(Debug, Default)]
pub struct Notifications {
    pub listeners: HashMap<u32, Listener>,
    notified_system: bool,
    notified_live: bool,
    notified_xmp: bool,
}

impl Notifications {
    /// KernelState::RegisterNotifyListener: the first listener of each area gets the startup notifications, when
    /// `startup` is set (XENON_STARTUP_NOTIFICATIONS; see docs/HANDOFF.md).
    pub fn register(&mut self, mem: &mut dyn GuestMem, handle: u32, mut listener: Listener, is_dashboard: bool, startup: bool) {
        if !startup {
            self.listeners.insert(handle, listener);
            return;
        }
        if !self.notified_system && listener.mask & NOTIFY_SYSTEM != 0 {
            self.notified_system = true;
            listener.enqueue(mem, SYSTEM_UI, 0); // XamIsUIActive: no system UI
            listener.enqueue(mem, SYSTEM_SIGN_IN_CHANGED, 1);
            if is_dashboard {
                listener.enqueue(mem, SYSTEM_STORAGE_DEVICES_CHANGED, 0);
            }
        }
        if !self.notified_live && listener.mask & NOTIFY_LIVE != 0 {
            self.notified_live = true;
            listener.enqueue(mem, LIVE_CONNECTION_CHANGED, LOGON_DISCONNECTED);
            listener.enqueue(mem, LIVE_LINK_STATE_CHANGED, 0);
        }
        if !self.notified_xmp && listener.mask & NOTIFY_XMP != 0 {
            self.notified_xmp = true;
            // the title controls playback (AudioMediaPlayer defaults: game client, game controller)
            listener.enqueue(mem, XMP_PLAYBACK_CONTROLLER_CHANGED, 1);
        }
        self.listeners.insert(handle, listener);
    }

    /// KernelState::BroadcastNotification.
    pub fn broadcast(&mut self, mem: &mut dyn GuestMem, id: u32, data: u32) {
        for listener in self.listeners.values_mut() {
            listener.enqueue(mem, id, data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;
    use crate::kernel::objects::{init_header, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE};

    fn listener(mem: &mut FakeMem, event: u32, mask: u64) -> Listener {
        init_header(mem, event as u64, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE, 0);
        Listener { mask, max_version: 10, event, queue: VecDeque::new() }
    }

    #[test]
    fn startup_notifications_go_to_the_first_listener_of_each_area() {
        let mut m = FakeMem::default();
        let mut n = Notifications::default();
        let a = listener(&mut m, 0x100, NOTIFY_SYSTEM);
        n.register(&mut m, 1, a, true, true);
        assert_eq!(n.listeners[&1].queue, VecDeque::from(vec![(SYSTEM_UI, 0), (SYSTEM_SIGN_IN_CHANGED, 1), (SYSTEM_STORAGE_DEVICES_CHANGED, 0)]));
        assert_eq!(m.read_u32(0x104), 1, "the wait handle is signaled");
        // the dash's second listener (mask 0x23): Live and XMP startup, no second system set
        let b = listener(&mut m, 0x200, 0x23);
        n.register(&mut m, 2, b, true, true);
        assert_eq!(n.listeners[&2].queue, VecDeque::from(vec![(LIVE_CONNECTION_CHANGED, 0x0015_10F1), (LIVE_LINK_STATE_CHANGED, 0), (XMP_PLAYBACK_CONTROLLER_CHANGED, 1)]));
    }

    #[test]
    fn dequeue_resets_the_event_when_empty_and_masks_filter() {
        let mut m = FakeMem::default();
        let mut l = listener(&mut m, 0x100, NOTIFY_SYSTEM);
        l.enqueue(&mut m, LIVE_CONNECTION_CHANGED, 5); // Live area: masked out
        assert!(l.queue.is_empty());
        l.enqueue(&mut m, SYSTEM_UI, 0);
        l.enqueue(&mut m, SYSTEM_DASH_CONTEXT_CHANGED, 0);
        assert_eq!(l.dequeue(&mut m, SYSTEM_DASH_CONTEXT_CHANGED), Some((SYSTEM_DASH_CONTEXT_CHANGED, 0)));
        assert_eq!(m.read_u32(0x104), 1);
        assert_eq!(l.dequeue(&mut m, 0), Some((SYSTEM_UI, 0)));
        assert_eq!((m.read_u32(0x104), l.dequeue(&mut m, 0)), (0, None));
    }
}
