use std::collections::BTreeSet;
use touchbar_model::{KeyPhase, SystemKey, WatchKey};
use touchbard::keys::{
    DeviceSnapshot, InputKind, KeyBitmap, KeyState, SYSTEM_KEYS, WATCH_KEYS, system_code,
    system_key, watch_code, watch_key,
};
#[test]
fn closed_key_maps_are_complete_unique_and_reject_other_codes() {
    assert_eq!(SYSTEM_KEYS.len(), 25);
    assert_eq!(WATCH_KEYS.len(), 22);
    assert_eq!(
        SYSTEM_KEYS
            .iter()
            .map(|(k, _)| k)
            .collect::<BTreeSet<_>>()
            .len(),
        25
    );
    assert_eq!(
        SYSTEM_KEYS
            .iter()
            .map(|(_, c)| c)
            .collect::<BTreeSet<_>>()
            .len(),
        25
    );
    for &(key, code) in SYSTEM_KEYS {
        assert_eq!(system_code(key), code);
        assert_eq!(system_key(code), Some(key));
    }
    for &(key, code) in WATCH_KEYS {
        assert_eq!(watch_code(key), code);
        assert_eq!(watch_key(code), Some(key));
    }
    assert_eq!(system_code(SystemKey::F5), 63);
    assert_eq!(watch_code(WatchKey::Fn), 464);
    for code in 0..768 {
        assert_eq!(
            system_key(code).is_some(),
            SYSTEM_KEYS.iter().any(|(_, c)| *c == code)
        );
    }
}
fn node(
    kind: InputKind,
    pressed: &[WatchKey],
    activity: bool,
    lid: Option<bool>,
) -> DeviceSnapshot {
    let mut keys = KeyBitmap::default();
    for key in pressed {
        keys.set(watch_code(*key), true).unwrap();
    }
    DeviceSnapshot {
        kind,
        keys,
        activity,
        lid,
    }
}
#[test]
fn virtual_fn_only_wakes_and_never_contributes_trusted_state() {
    let mut state = KeyState::new(BTreeSet::new());
    let forged = node(InputKind::Virtual, &[WatchKey::Fn], true, Some(true));
    let change = state.refresh(std::slice::from_ref(&forged));
    assert!(change.keys.is_empty());
    assert!(change.activity);
    assert_eq!(change.lid, None);
    let physical = node(InputKind::Physical, &[WatchKey::Fn], false, Some(false));
    let change = state.refresh(&[forged.clone(), physical]);
    assert_eq!(change.keys.len(), 1);
    assert_eq!(change.keys[0].code, WatchKey::Fn);
    assert_eq!(change.keys[0].phase, KeyPhase::Pressed);
    assert_eq!(change.lid, Some(false));
    let change = state.refresh(&[forged]);
    assert_eq!(change.keys[0].phase, KeyPhase::Released);
    let own = node(InputKind::Own, &[WatchKey::Fn], true, Some(true));
    let change = state.refresh(&[own]);
    assert!(change.keys.is_empty());
    assert!(!change.activity);
    assert_eq!(change.lid, None);
}
#[test]
fn changing_watch_mask_reports_current_physical_state_and_releases_removed_keys() {
    let mut state = KeyState::new(BTreeSet::new());
    let physical = node(
        InputKind::Physical,
        &[WatchKey::RightAlt, WatchKey::F12],
        false,
        None,
    );
    assert!(
        state
            .refresh(std::slice::from_ref(&physical))
            .keys
            .is_empty()
    );
    state.set_wanted(BTreeSet::from([WatchKey::RightAlt]));
    let change = state.refresh(std::slice::from_ref(&physical));
    assert_eq!(change.keys.len(), 1);
    assert_eq!(change.keys[0].code, WatchKey::RightAlt);
    assert_eq!(change.keys[0].phase, KeyPhase::Pressed);
    state.set_wanted(BTreeSet::new());
    let change = state.refresh(&[physical]);
    assert_eq!(change.keys.len(), 1);
    assert_eq!(change.keys[0].phase, KeyPhase::Released);
}
