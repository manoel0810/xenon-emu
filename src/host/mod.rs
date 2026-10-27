//! Host abstraction layer (`xe-host`).
//!
//! Decouples the guest Xbox 360 kernel and subsystems from the host operating system.
//! Subsystems can interact with the host environment either through:
//! - `HostOffline`: Deterministic, safe, isolated backend for automated testing and CI.
//! - `HostReal`: OS-backed backend providing real storage, system clock, and hardware XInput.

pub mod traits;
pub mod offline;
pub mod real;

pub use traits::*;
pub use offline::*;
pub use real::*;

/// Returns the default host environment (`HostOffline` for maximum determinism and safety).
pub fn default_host() -> Box<dyn Host> {
    Box::new(HostOffline::new())
}

/// Returns a real OS-backed host environment.
pub fn real_host() -> Box<dyn Host> {
    Box::new(HostReal::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::SeekFrom;

    #[test]
    fn test_host_offline_storage_read_write() {
        let mut host = HostOffline::new();
        let storage = host.storage();

        // Create new file
        let mut file = storage.open("test.txt", OpenMode::Create).expect("create file");
        let sample_data = b"Hello Xbox 360 Host!";
        let written = file.write(sample_data).expect("write bytes");
        assert_eq!(written, sample_data.len());
        assert_eq!(file.size(), sample_data.len() as u64);

        // Seek and read
        file.seek(SeekFrom::Start(6)).expect("seek to 6");
        let mut read_buf = [0u8; 8];
        let bytes_read = file.read(&mut read_buf).expect("read 8 bytes");
        assert_eq!(bytes_read, 8);
        assert_eq!(&read_buf, b"Xbox 360");
    }

    #[test]
    fn test_host_offline_network_and_profile() {
        let mut host = HostOffline::new();
        assert_eq!(host.net().link_status(), LinkStatus::Disconnected);
        assert!(host.net().socket(2, 1, 6).is_err());

        let users = host.profile().signed_in_users();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].slot, 0);
        assert_eq!(users[0].gamertag, "Xbox Player");
        assert_eq!(users[0].signin_state, 1);
    }

    #[test]
    fn test_host_offline_time_and_audio() {
        let mut host = HostOffline::new();
        let t1 = host.time().now_filetime();
        let t2 = host.time().now_filetime();
        assert!(t2 > t1);
        assert_eq!(host.time().query_performance_frequency(), 50_000_000);

        let pcm_data = [100i16, 200, -100, -200];
        host.audio().submit_pcm(&pcm_data);
        assert_eq!(host.audio.total_samples_submitted, 4);
    }

    #[test]
    fn test_host_real_time_and_input() {
        let mut host = HostReal::new();
        let ft = host.time().now_filetime();
        // Modern FILETIME should be well past year 2020 (>= 132_000_000_000_000_000)
        assert!(ft > 132_000_000_000_000_000);
        assert_eq!(host.time().query_performance_frequency(), 50_000_000);

        // Polling pad shouldn't panic even without controller
        let pad = host.input().poll_pad(0);
        assert!(pad.connected); // Slot 0 has virtual keyboard gamepad connected by default
    }

    #[test]
    fn test_host_offline_virtual_pad_injection() {
        let mut host = HostOffline::new();
        let pad = host.input().poll_pad(0);
        assert!(pad.connected);
        assert_eq!(pad.w_buttons, 0);

        // Inject virtual keyboard input (e.g. A button + thumb stick)
        let virtual_pad = PadState {
            w_buttons: 0x1000, // XINPUT_GAMEPAD_A
            b_left_trigger: 128,
            b_right_trigger: 0,
            s_thumb_lx: 24000,
            s_thumb_ly: -12000,
            s_thumb_rx: 0,
            s_thumb_ry: 0,
            connected: true,
        };
        host.input().set_virtual_pad(0, virtual_pad);

        let polled = host.input().poll_pad(0);
        assert!(polled.connected);
        assert_eq!(polled.w_buttons, 0x1000);
        assert_eq!(polled.b_left_trigger, 128);
        assert_eq!(polled.s_thumb_lx, 24000);
        assert_eq!(polled.s_thumb_ly, -12000);
    }

    #[test]
    fn test_host_real_virtual_pad_merging() {
        let mut host = HostReal::new();
        let pad_state = PadState {
            w_buttons: 0x2000 | 0x0010, // B + START
            b_left_trigger: 255,
            b_right_trigger: 255,
            s_thumb_lx: -15000,
            s_thumb_ly: 15000,
            s_thumb_rx: 0,
            s_thumb_ry: 0,
            connected: true,
        };
        host.input().set_virtual_pad(0, pad_state);

        let polled = host.input().poll_pad(0);
        assert!(polled.connected);
        assert_eq!(polled.w_buttons & (0x2000 | 0x0010), 0x2000 | 0x0010);
        assert_eq!(polled.b_left_trigger, 255);
        assert_eq!(polled.b_right_trigger, 255);
    }

    #[test]
    fn test_host_real_net_opt_in() {
        // Default / offline mode
        let mut net_offline = RealNet::with_online(false);
        assert_eq!(net_offline.link_status(), LinkStatus::Disconnected);
        assert!(net_offline.socket(2, 1, 6).is_err());
        assert!(net_offline.dns_resolve("localhost").is_err());

        // Opt-in online mode
        let mut net_online = RealNet::with_online(true);
        match net_online.link_status() {
            LinkStatus::Connected { ip, subnet, .. } => {
                assert_ne!(ip, [0, 0, 0, 0]);
                assert_eq!(subnet, [255, 255, 255, 0]);
            }
            LinkStatus::Disconnected => panic!("Expected Connected in online mode"),
        }

        // DNS resolution of localhost
        let resolved = net_online.dns_resolve("localhost");
        assert!(resolved.is_ok());
        let ips = resolved.unwrap();
        assert!(ips.contains(&[127, 0, 0, 1]));

        // UDP socket creation, send and close
        let sock = net_online.socket(2, 2, 17).expect("create UDP socket");
        assert!(sock > 0);
        assert!(net_online.close_socket(sock).is_ok());
    }
}
