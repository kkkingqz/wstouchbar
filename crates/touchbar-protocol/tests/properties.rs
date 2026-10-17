use proptest::prelude::*;
use std::num::NonZeroU32;
use touchbar_model::{KeyPhase, SystemKey, TouchPhase, WatchKey};
use touchbar_protocol::*;
proptest! {
    #[test]
    fn arbitrary_records_never_panic(data in prop::collection::vec(any::<u8>(),0..100), record in any::<[u8;32]>()) {
        let _ = SessionMessage::decode(&record);
        let _ = DaemonMessage::decode(&record);
        let _ = SessionMessage::decode(&data);
        let _ = DaemonMessage::decode(&data);
        let _ = SwapchainHeader::decode(&data);
    }
    #[test]
    fn session_values_roundtrip(index in 0u32..3, frame in any::<u64>(), limit in any::<u16>(), enabled in any::<bool>(), key in 0usize..12, version in any::<u16>(), rect in (any::<u16>(), any::<u16>(),1u16..=65535,1u16..=65535)) {
        let watch = [WatchKey::F1,WatchKey::F2,WatchKey::F3,WatchKey::F4,WatchKey::F5,WatchKey::F6,WatchKey::F7,WatchKey::F8,WatchKey::F9,WatchKey::F10,WatchKey::F11,WatchKey::F12];
        let system = [SystemKey::F1,SystemKey::F2,SystemKey::F3,SystemKey::F4,SystemKey::F5,SystemKey::F6,SystemKey::F7,SystemKey::F8,SystemKey::F9,SystemKey::F10,SystemKey::F11,SystemKey::F12];
        for message in [SessionMessage::Hello{version}, SessionMessage::FrameReady{index,frame}, SessionMessage::Backlight{limit}, SessionMessage::WatchKey{key:watch[key],enabled},SessionMessage::Key{key:system[key],phase:if enabled {KeyPhase::Pressed} else {KeyPhase::Released}},SessionMessage::FrameDamage(DamageRect{x:rect.0,y:rect.1,width:rect.2,height:rect.3})] {
            prop_assert_eq!(SessionMessage::decode(&message.encode().unwrap()).unwrap(),message);
        }
    }
    #[test]
    fn daemon_values_roundtrip(index in 0u32..3, frame in any::<u64>(), commit_us in 0u32..=i32::MAX as u32, contact in 1u32..=u32::MAX, coords in (any::<i32>(),any::<i32>()), caps in any::<u32>(), width in 1u32..=MAX_DIMENSION, height in 1u32..=MAX_DIMENSION, mm in (1u32..=u32::MAX,1u32..=u32::MAX)) {
        for message in [DaemonMessage::Welcome{caps,width,height,width_mm100:mm.0,height_mm100:mm.1},DaemonMessage::BufferReleased{index,frame},DaemonMessage::FramePresented{index,frame,commit_us},DaemonMessage::Touch{phase:TouchPhase::Down,contact:NonZeroU32::new(contact).unwrap(),x:coords.0,y:coords.1}] {
            prop_assert_eq!(DaemonMessage::decode(&message.encode().unwrap()).unwrap(),message);
        }
    }
    #[test]
    fn valid_swapchain_roundtrip(width in 1u32..=MAX_DIMENSION, height in 1u32..=MAX_DIMENSION, rotated in any::<bool>(), count in 2u16..=3, damage in any::<bool>()) {
        let (pw,ph) = if rotated {(height,width)} else {(width,height)};
        let header = SwapchainHeader { logical_width:width,logical_height:height,physical_width:pw,physical_height:ph,format:XRGB8888,pitch:pw*4,buffer_size:u64::from(pw)*u64::from(ph)*4,buffer_count:count,frame_damage:damage };
        prop_assert_eq!(SwapchainHeader::decode(&header.encode().unwrap()).unwrap(),header);
    }
}
