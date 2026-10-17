use touchbar_core::*;
use touchbar_model::*;
use touchbar_session::diagnostics::*;
#[test]
fn repeats_log_immediately_then_summarize_once_per_minute() {
    let mut logs = Diagnostics::default();
    assert_eq!(logs.error(Timestamp(0), None, "broken").len(), 1);
    for now in [1, 100, 999, 59_999] {
        assert!(logs.error(Timestamp(now), None, "broken").is_empty());
    }
    assert!(logs.tick(Timestamp(59_999)).is_empty());
    let summary = logs.tick(Timestamp(60_000));
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].suppressed, 4);
    assert!(logs.tick(Timestamp(60_001)).is_empty());
}
#[test]
fn guest_rate_limit_is_per_instance_and_reports_drops() {
    let mut logs = Diagnostics::default();
    let a = InstanceId {
        number: 1,
        generation: Generation(1),
    };
    let b = InstanceId {
        number: 2,
        generation: Generation(1),
    };
    for _ in 0..100 {
        assert_eq!(logs.guest(Timestamp(0), a, "hello").len(), 1);
    }
    assert!(logs.guest(Timestamp(0), a, "spam").is_empty());
    assert_eq!(logs.guest(Timestamp(0), b, "hello").len(), 1);
    assert_eq!(logs.tick(Timestamp(1000))[0].suppressed, 1);
    assert_eq!(logs.guest(Timestamp(1000), a, "new second").len(), 1);
}
#[test]
fn dumps_rotate_ten_files_manual_requests_bypass_fault_throttle() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = DumpStore::new(dir.path().into());
    let dump = Recorder::new(&Core::new(CoreConfig::default())).dump();
    assert!(store.fault(Timestamp(0), &dump).unwrap().is_some());
    assert!(store.fault(Timestamp(1), &dump).unwrap().is_none());
    for _ in 0..15 {
        store.manual(&dump).unwrap();
    }
    let files = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(files.len(), 10);
    for path in files {
        ReplayDump::from_json(&std::fs::read_to_string(path).unwrap())
            .unwrap()
            .replay()
            .unwrap();
    }
    assert!(store.fault(Timestamp(60_000), &dump).unwrap().is_some());
}
#[test]
fn guest_limit_cannot_double_burst_at_a_window_boundary() {
    let mut logs = Diagnostics::default();
    let instance = InstanceId {
        number: 1,
        generation: Generation(1),
    };
    assert_eq!(logs.guest(Timestamp(0), instance, "first").len(), 1);
    for _ in 0..99 {
        assert_eq!(logs.guest(Timestamp(999), instance, "burst").len(), 1);
    }
    let accepted = (0..100)
        .filter(|_| !logs.guest(Timestamp(1000), instance, "boundary").is_empty())
        .count();
    assert_eq!(accepted, 1);
}
#[test]
fn warning_severity_survives_suppression() {
    let mut logs = Diagnostics::default();
    assert!(matches!(
        logs.report(Timestamp(0), None, "warning", Level::Warn)[0].level,
        Level::Warn
    ));
    logs.report(Timestamp(1), None, "warning", Level::Warn);
    assert!(matches!(logs.tick(Timestamp(60000))[0].level, Level::Warn));
}
