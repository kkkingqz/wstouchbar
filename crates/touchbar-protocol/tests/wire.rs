use std::num::NonZeroU32;
use touchbar_model::{KeyPhase, SystemKey, TouchPhase, WatchKey};
use touchbar_protocol::*;

fn bytes(kind: u16, value: u32, payload: u64, x: i32, y: i32) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..8].copy_from_slice(b"TBARMSG2");
    bytes[8..10].copy_from_slice(&2u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&kind.to_le_bytes());
    bytes[12..16].copy_from_slice(&value.to_le_bytes());
    bytes[16..24].copy_from_slice(&payload.to_le_bytes());
    bytes[24..28].copy_from_slice(&x.to_le_bytes());
    bytes[28..32].copy_from_slice(&y.to_le_bytes());
    bytes
}
#[test]
fn session_messages_follow_v2_field_layout_and_directions() {
    let cases = [
        (SessionMessage::Hello { version: 2 }, bytes(10, 2, 0, 0, 0)),
        (
            SessionMessage::FrameReady {
                index: 2,
                frame: u64::MAX,
            },
            bytes(1, 2, u64::MAX, 0, 0),
        ),
        (
            SessionMessage::FrameDamage(DamageRect {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            }),
            bytes(9, 0, 1 | 2 << 16 | 3 << 32 | 4 << 48, 0, 0),
        ),
        (
            SessionMessage::Key {
                key: SystemKey::F5,
                phase: KeyPhase::Pressed,
            },
            bytes(8, 25, 0, 1, 0),
        ),
        (
            SessionMessage::Key {
                key: SystemKey::Esc,
                phase: KeyPhase::Released,
            },
            bytes(8, 20, 0, 0, 0),
        ),
        (
            SessionMessage::WatchKey {
                key: WatchKey::Fn,
                enabled: true,
            },
            bytes(16, 1, 0, 1, 0),
        ),
        (
            SessionMessage::WatchKey {
                key: WatchKey::Fn,
                enabled: false,
            },
            bytes(16, 1, 0, 0, 0),
        ),
        (
            SessionMessage::Backlight { limit: u16::MAX },
            bytes(17, 65535, 0, 0, 0),
        ),
    ];
    for (message, expected) in cases {
        assert_eq!(message.encode().unwrap(), expected);
        assert_eq!(SessionMessage::decode(&expected).unwrap(), message);
        assert!(DaemonMessage::decode(&expected).is_err());
    }
}
#[test]
fn daemon_messages_follow_v2_field_layout_and_directions() {
    let cases = [
        (
            DaemonMessage::Welcome {
                caps: u32::MAX,
                width: 2008,
                height: 60,
                width_mm100: 33467,
                height_mm100: 1000,
            },
            bytes(11, u32::MAX, 33467 | 1000 << 32, 2008, 60),
        ),
        (
            DaemonMessage::Revoked(RevokedReason::Replaced),
            bytes(12, 3, 0, 0, 0),
        ),
        (
            DaemonMessage::ModeChanged {
                mode: Mode::Native,
                reason: ModeReason::Locked,
            },
            bytes(13, 0, 2, 0, 0),
        ),
        (
            DaemonMessage::Touch {
                phase: TouchPhase::Cancel,
                contact: NonZeroU32::new(u32::MAX).unwrap(),
                x: i32::MIN,
                y: i32::MAX,
            },
            bytes(14, 3, u64::from(u32::MAX), i32::MIN, i32::MAX),
        ),
        (
            DaemonMessage::KeyEvent {
                key: WatchKey::Fn,
                phase: KeyPhase::Pressed,
            },
            bytes(15, 1, 0, 1, 0),
        ),
        (
            DaemonMessage::BufferReleased { index: 0, frame: 5 },
            bytes(2, 0, 5, 0, 0),
        ),
        (
            DaemonMessage::FramePresented {
                index: 1,
                frame: 6,
                commit_us: 1234,
            },
            bytes(18, 1, 6, 1234, 0),
        ),
    ];
    for (message, expected) in cases {
        assert_eq!(message.encode().unwrap(), expected);
        assert_eq!(DaemonMessage::decode(&expected).unwrap(), message);
        assert!(SessionMessage::decode(&expected).is_err());
    }
}
#[test]
fn malformed_messages_reject_lengths_ranges_reserved_fields_and_versions() {
    for raw in [
        bytes(1, 3, 0, 0, 0),
        bytes(8, 999, 0, 1, 0),
        bytes(8, 21, 0, 2, 0),
        bytes(8, 21, 1, 0, 0),
        bytes(9, 0, 0, 0, 0),
        bytes(16, 1, 0, 2, 0),
        bytes(16, 999, 0, 1, 0),
        bytes(17, 65536, 0, 0, 0),
        bytes(10, 2, 0, 0, 1),
        bytes(65535, 0, 0, 0, 0),
    ] {
        assert!(SessionMessage::decode(&raw).is_err(), "accepted {raw:?}");
    }
    for raw in [
        bytes(14, 0, 0, 0, 0),
        bytes(14, 0, u64::from(u32::MAX) + 1, 0, 0),
        bytes(14, 4, 1, 0, 0),
        bytes(11, 0, 1, 2008, 60),
        bytes(11, 0, 1 | 1 << 32, 0, 60),
        bytes(18, 0, 1, -1, 0),
        bytes(18, 0, 1, 0, 1),
        bytes(12, 6, 0, 0, 0),
        bytes(13, 2, 0, 0, 0),
        bytes(13, 1, 5, 0, 0),
        bytes(15, 1, 0, 2, 0),
    ] {
        assert!(DaemonMessage::decode(&raw).is_err(), "accepted {raw:?}");
    }
    let valid = bytes(10, 2, 0, 0, 0);
    for length in 0..32 {
        assert!(SessionMessage::decode(&valid[..length]).is_err());
    }
    assert!(SessionMessage::decode(&[0; 33]).is_err());
    for offset in [0, 8] {
        let mut bad = valid;
        bad[offset] ^= 1;
        assert!(SessionMessage::decode(&bad).is_err());
    }
}
#[test]
fn swapchain_preserves_v1_layout_with_v2_magic_and_validates_geometry() {
    let info = SwapchainHeader {
        logical_width: 2008,
        logical_height: 60,
        physical_width: 60,
        physical_height: 2008,
        format: XRGB8888,
        pitch: 256,
        buffer_size: 514048,
        buffer_count: 3,
        frame_damage: true,
    };
    let mut expected = [0; 48];
    expected[..8].copy_from_slice(b"TBARHWD2");
    expected[8..10].copy_from_slice(&2u16.to_le_bytes());
    expected[10..12].copy_from_slice(&3u16.to_le_bytes());
    for (offset, value) in [
        (12, 2008u32),
        (16, 60),
        (20, 60),
        (24, 2008),
        (28, 0x34325258),
        (32, 256),
        (36, 1),
    ] {
        expected[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    expected[40..48].copy_from_slice(&514048u64.to_le_bytes());
    assert_eq!(info.encode().unwrap(), expected);
    assert_eq!(SwapchainHeader::decode(&expected).unwrap(), info);
    assert_eq!(info.transform(), Some(OutputTransform::QuarterTurn));
    for bad in [
        SwapchainHeader {
            buffer_count: 1,
            ..info
        },
        SwapchainHeader {
            logical_width: 0,
            ..info
        },
        SwapchainHeader {
            physical_width: 16385,
            ..info
        },
        SwapchainHeader { pitch: 239, ..info },
        SwapchainHeader {
            buffer_size: 514047,
            ..info
        },
        SwapchainHeader { format: 0, ..info },
        SwapchainHeader {
            logical_height: 61,
            ..info
        },
    ] {
        assert!(bad.encode().is_err());
    }
    for length in 0..48 {
        assert!(SwapchainHeader::decode(&expected[..length]).is_err());
    }
    for offset in [0, 8, 36] {
        let mut bad = expected;
        bad[offset] ^= 128;
        assert!(SwapchainHeader::decode(&bad).is_err());
    }
}
#[test]
fn every_closed_reason_phase_and_key_code_roundtrips() {
    for key in (1..=12).chain(20..=32) {
        for phase in 0..=1 {
            let raw = bytes(8, key, 0, phase, 0);
            assert_eq!(SessionMessage::decode(&raw).unwrap().encode().unwrap(), raw);
        }
    }
    for key in (1..=10).chain(21..=32) {
        for phase in 0..=1 {
            let raw = bytes(15, key, 0, phase, 0);
            assert_eq!(DaemonMessage::decode(&raw).unwrap().encode().unwrap(), raw);
        }
    }
    for reason in 1..=5 {
        let raw = bytes(12, reason, 0, 0, 0);
        assert_eq!(DaemonMessage::decode(&raw).unwrap().encode().unwrap(), raw);
    }
    for mode in 0..=1 {
        for reason in 0..=4 {
            let raw = bytes(13, mode, reason, 0, 0);
            assert_eq!(DaemonMessage::decode(&raw).unwrap().encode().unwrap(), raw);
        }
    }
    for phase in 0..=3 {
        let raw = bytes(14, phase, 1, 0, 0);
        assert_eq!(DaemonMessage::decode(&raw).unwrap().encode().unwrap(), raw);
    }
}
#[test]
fn all_reserved_fields_are_enforced() {
    for (raw, offsets) in [
        (bytes(10, 2, 0, 0, 0), vec![16, 24, 28]),
        (bytes(1, 0, 0, 0, 0), vec![24, 28]),
        (bytes(8, 1, 0, 0, 0), vec![16, 28]),
        (bytes(16, 1, 0, 0, 0), vec![16, 28]),
        (bytes(17, 0, 0, 0, 0), vec![16, 24, 28]),
        (bytes(9, 0, 1 << 32 | 1 << 48, 0, 0), vec![12, 24, 28]),
    ] {
        for offset in offsets {
            let mut bad = raw;
            bad[offset] = 1;
            assert!(SessionMessage::decode(&bad).is_err(), "{bad:?}");
        }
    }
    for (raw, offsets) in [
        (bytes(12, 1, 0, 0, 0), vec![16, 24, 28]),
        (bytes(2, 0, 0, 0, 0), vec![24, 28]),
        (bytes(15, 1, 0, 0, 0), vec![16, 28]),
        (bytes(13, 1, 0, 0, 0), vec![24, 28]),
    ] {
        for offset in offsets {
            let mut bad = raw;
            bad[offset] = 1;
            assert!(DaemonMessage::decode(&bad).is_err(), "{bad:?}");
        }
    }
}
