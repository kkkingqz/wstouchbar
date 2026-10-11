use touchbar_core::*;
use touchbar_testkit::strategies::GeneratedScenario;
use touchbar_testkit::*;

#[test]
fn stored_json_scenarios_replay_and_preserve_invariants() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regressions");
    let mut count = 0;
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let json = std::fs::read_to_string(&path).unwrap();
        if serde_json::from_str::<serde_json::Value>(&json)
            .unwrap()
            .get("profiles")
            .is_some()
        {
            let case: GeneratedScenario = serde_json::from_str(&json).unwrap();
            let h = case.run();
            assert!(
                h.invariant_error.is_none(),
                "{}: {:?}",
                path.display(),
                h.invariant_error
            );
            h.finish_invariant_checks().unwrap();
            assert_eq!(
                ReplayDump::from_json(&h.dump().to_json().unwrap())
                    .unwrap()
                    .replay()
                    .unwrap()
                    .core
                    .snapshot(),
                h.snapshot()
            );
        } else {
            let dump = ReplayDump::from_json(&json).unwrap();
            let mut core = Core::restore(dump.checkpoint.clone()).unwrap();
            let mut checker = InvariantChecker::new(core.snapshot());
            for entry in &dump.inputs {
                assert_eq!(
                    core.handle(entry.at, entry.input.clone()),
                    entry.outputs,
                    "{}",
                    path.display()
                );
                checker.observe(entry, &core.snapshot()).unwrap();
            }
            checker.finish().unwrap();
        }
        count += 1;
    }
    assert!(count > 0, "regression fixtures must not be empty");
}
