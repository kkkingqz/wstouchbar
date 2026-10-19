use touchbar_model::Timestamp;
use touchbar_session::{Clock, ManualClock, cli::*, locale_from};
fn parse(args: &[&str]) -> Result<Cli, String> {
    Cli::parse(args.iter().map(|s| s.to_string()))
}
#[test]
fn headless_simulator_options_and_invalid_combinations() {
    let cli = parse(&[
        "--headless",
        "2008x60@6.2ppmm",
        "--desktop=fake",
        "--effects=fake",
        "--plugins",
        "/tmp/test-plugins",
    ])
    .unwrap();
    assert_eq!(cli.headless.unwrap().width_px, 2008);
    assert!(cli.fake_effects);
    assert_eq!(cli.plugins.unwrap().to_str().unwrap(), "/tmp/test-plugins");
    assert!(
        parse(&["--simulator", "--surface", "1600x48@5ppmm"])
            .unwrap()
            .simulator
    );
    for args in [
        vec!["--headless", "0x60@6ppmm"],
        vec!["--headless", "4x6@NaNppmm"],
        vec!["--headless", "9000x60@6ppmm"],
        vec!["--simulator", "--headless", "2008x60@6ppmm"],
        vec!["--surface", "2008x60@6ppmm"],
        vec!["--desktop=bad"],
        vec!["--effects=native-bad"],
        vec!["--plugins"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}
#[test]
fn locale_precedence_and_monotonic_manual_time() {
    assert_eq!(
        locale_from(|key| match key {
            "LC_ALL" => Some("".into()),
            "LC_MESSAGES" => Some("uk_UA.UTF-8".into()),
            "LANG" => Some("en_US.UTF-8".into()),
            _ => None,
        }),
        "uk_UA.UTF-8"
    );
    assert_eq!(locale_from(|_| None), "C");
    let clock = ManualClock::default();
    clock.advance(17);
    assert_eq!(clock.now(), Timestamp(17));
    assert!(clock.set(Timestamp(16)).is_err());
}
#[test]
fn touchbard_socket_override_is_absolute_and_explicit() {
    assert_eq!(
        parse(&["--touchbard-socket=/tmp/touchbar-test.sock"])
            .unwrap()
            .touchbard_socket
            .unwrap(),
        std::path::Path::new("/tmp/touchbar-test.sock")
    );
    assert!(parse(&["--touchbard-socket=relative.sock"]).is_err());
    assert!(parse(&["--touchbard-socket"]).is_err());
}
