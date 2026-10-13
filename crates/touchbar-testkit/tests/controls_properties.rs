mod common;
use proptest::test_runner::{Config, TestCaseError, TestRunner};
use touchbar_testkit::{strategies::scenario_strategy, *};
#[test]
fn controls_generated_sequences_preserve_all_twelve_invariants() {
    let (plugin, host) = common::controls_host();
    let host = std::cell::RefCell::new(host);
    let config = Config {
        cases: std::env::var("PROPTEST_CONTROLS_CASES")
            .map(|s| s.parse().unwrap())
            .unwrap_or(200),
        source_file: Some(file!()),
        failure_persistence: Some(Box::new(
            proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"),
        )),
        ..Default::default()
    };
    let coverage = std::cell::RefCell::new([0usize; 4]);
    let mut runner = TestRunner::new(config);
    runner
        .run(&scenario_strategy(), |case| {
            let h = common::run_controls(
                &case,
                host.borrow_mut()
                    .take()
                    .unwrap_or_else(|| common::controls_host().1.unwrap()),
                &plugin,
            );
            let states = h.invariant_coverage();
            let mut counts = coverage.borrow_mut();
            counts[0] += 1;
            counts[1] += usize::from(states.panel);
            counts[2] += usize::from(states.pending);
            counts[3] += usize::from(states.faulted);
            drop(counts);
            let result = h
                .finish_invariant_checks()
                .map_err(|e| TestCaseError::fail(e.to_string()));
            assert!(h.host_errors.is_empty());
            let replay = ReplayDump::from_json(&h.dump().to_json().unwrap())
                .unwrap()
                .replay()
                .unwrap();
            assert_eq!(replay.core.snapshot(), h.snapshot());
            *host.borrow_mut() = Some(h.logic);
            result
        })
        .unwrap();
    let counts = coverage.into_inner();
    eprintln!(
        "controls coverage: {} cases, {} panels, {} pending, {} faults",
        counts[0], counts[1], counts[2], counts[3]
    );
    for count in &counts[1..] {
        assert!(
            *count * 100 >= counts[0] * 5,
            "state coverage below 5%: {counts:?}"
        );
    }
}
