use touchbar_bindings::{BindingEngine, BindingEvent, KeyInput, config::RecoveryAction};
use touchbar_model::{KeyPhase, Timestamp, WatchKey};
use touchbard::config::*;
#[test]
fn shipped_defaults_keep_driver_timeouts_and_fn_eight_second_recovery() {
    let config = Config::default();
    assert_eq!(config.session_grace_ms, 2000);
    assert_eq!(config.dim_timeout, None);
    assert_eq!(config.idle_timeout, None);
    assert_eq!(
        config.backlight,
        BacklightLevels {
            active: 2,
            dimmed: 1
        }
    );
    let mut engine = BindingEngine::new(config.bindings).unwrap();
    assert!(
        engine
            .handle(
                Timestamp(0),
                KeyInput {
                    code: WatchKey::Fn,
                    phase: KeyPhase::Pressed
                }
            )
            .is_empty()
    );
    assert!(engine.advance(Timestamp(7999)).is_empty());
    assert!(matches!(
        engine.advance(Timestamp(8000)).as_slice(),
        [BindingEvent::Fired {
            action: RecoveryAction::ToggleRecovery,
            ..
        }]
    ));
}
#[test]
fn strict_config_rejects_unknown_keys_bad_levels_and_unrepresentable_timers() {
    for source in [
        "typo=1",
        "session-grace-ms=-1",
        "dim-timeout=-1",
        "dim-timeout=18446744073709552",
        "idle-timeout=18446744073709551615",
        "log='[bad'",
        "[backlight]\nactive=3",
        "[backlight]\ndimmed=3",
        "[backlight]\nactive=0",
        "[backlight]\nactive=1\ndimmed=2",
        "[backlight]\nextra=1",
        "[[binding]]\nid='x'\naction='unknown'",
        "[[binding]]\nid='x'\ntrigger={key='fn',gesture='press'}\naction='native'\nwhen={exists='app'}",
    ] {
        let error = ConfigFile::from_toml("touchbard.toml", source).unwrap_err();
        assert_eq!(error.file, std::path::Path::new("touchbard.toml"));
        assert!(error.line >= 1, "{error:?}");
    }
    assert!(ConfigFile::from_toml("touchbard.toml", &" ".repeat(256 * 1024 + 1)).is_err());
    assert!(
        ConfigFile::from_toml(
            "touchbard.toml",
            "session-grace-ms=0\ndim-timeout=0\nidle-timeout=0"
        )
        .is_ok()
    );
}
#[test]
fn admin_config_replaces_disables_and_adds_bindings_by_id_and_merges_scalars() {
    let base = ConfigFile::from_toml("base", DEFAULT_CONFIG).unwrap();
    let admin=ConfigFile::from_toml("admin","session-grace-ms=17\ndim-timeout=4\n[backlight]\ndimmed=0\n[[binding]]\nid='recovery'\ntrigger={key='fn',gesture='hold',ms=9000}\naction='native'\n[[binding]]\nid='exit'\ntrigger={key='esc',gesture='press'}\naction='display'").unwrap();
    let config = Config::merge(&base, Some(&admin)).unwrap();
    assert_eq!(config.session_grace_ms, 17);
    assert_eq!(config.dim_timeout, Some(4));
    assert_eq!(config.backlight.active, 2);
    assert_eq!(config.backlight.dimmed, 0);
    assert_eq!(config.bindings.len(), 2);
    assert_eq!(config.bindings[0].action, RecoveryAction::Native);
    let disabled =
        ConfigFile::from_toml("admin", "[[binding]]\nid='recovery'\ndisabled=true").unwrap();
    assert!(
        Config::merge(&base, Some(&disabled))
            .unwrap()
            .bindings
            .is_empty()
    );
}
#[test]
fn corrupt_admin_or_merged_conflict_preserves_whole_previous_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().join("usr.toml");
    let admin = temp.path().join("etc.toml");
    std::fs::write(&base, DEFAULT_CONFIG).unwrap();
    std::fs::write(&admin, "session-grace-ms=77").unwrap();
    let mut store = ConfigStore::default();
    store.reload(&base, &admin).unwrap();
    let previous = store.current().clone();
    std::fs::write(
        &base,
        DEFAULT_CONFIG.replace("log = \"info\"", "log = \"debug\""),
    )
    .unwrap();
    for source in [
        "[[broken",
        "[[binding]]\nid='other'\ntrigger={key='fn',gesture='hold',ms=8000}\naction='native'",
        "session-grace-ms=1\n[[binding]]\nid='recovery'\ndisabled=true\naction='native'",
    ] {
        std::fs::write(&admin, source).unwrap();
        let warning = store.reload(&base, &admin).unwrap_err();
        assert_eq!(warning.file, admin);
        assert_eq!(store.current(), &previous);
    }
    std::fs::remove_file(&admin).unwrap();
    store.reload(&base, &admin).unwrap();
    assert_eq!(store.current().session_grace_ms, 2000);
    std::fs::remove_file(&base).unwrap();
    store.reload(&base, &admin).unwrap();
    assert_eq!(store.current(), &Config::default());
}
#[test]
fn values_valid_in_separate_files_are_checked_together_after_merge() {
    let base = ConfigFile::from_toml(
        "base",
        "dim-timeout=10000000000000000\n[backlight]\nactive=1\ndimmed=1",
    )
    .unwrap();
    let timeout = ConfigFile::from_toml("admin", "idle-timeout=10000000000000000").unwrap();
    assert!(
        Config::merge(&base, Some(&timeout))
            .unwrap_err()
            .path
            .contains("idle-timeout")
    );
    let levels = ConfigFile::from_toml("admin", "[backlight]\ndimmed=2").unwrap();
    assert!(
        Config::merge(&base, Some(&levels))
            .unwrap_err()
            .path
            .contains("backlight.dimmed")
    );
}
