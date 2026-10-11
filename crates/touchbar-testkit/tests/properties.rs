use proptest::test_runner::{Config, TestCaseError, TestRunner};
use touchbar_testkit::strategies::scenario_strategy;
use touchbar_testkit::*;

#[test]
fn generated_sequences_preserve_all_twelve_invariants() {
    let mut config = Config::default();
    if std::env::var_os("PROPTEST_CASES").is_none() {
        config.cases = 2000;
    }
    config.source_file = Some(file!());
    config.failure_persistence = Some(Box::new(
        proptest::test_runner::FileFailurePersistence::WithSource("proptest-regressions"),
    ));
    let mut runner = TestRunner::new(config);
    let coverage = std::cell::RefCell::new([0usize; 4]);
    let result = runner.run(&scenario_strategy(), |case| {
        let h = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| case.run())) {
            Ok(h) => h,
            Err(error) => {
                let directory = regression_directory();
                std::fs::create_dir_all(&directory).unwrap();
                std::fs::write(
                    directory.join("panic.case.json"),
                    serde_json::to_string_pretty(&case).unwrap(),
                )
                .unwrap();
                let message = error
                    .downcast_ref::<String>()
                    .map(String::as_str)
                    .or_else(|| error.downcast_ref::<&str>().copied())
                    .unwrap_or("scenario panicked");
                return Err(TestCaseError::fail(message.to_owned()));
            }
        };
        let states = h.invariant_coverage();
        let (panel, pending, fault) = (states.panel, states.pending, states.faulted);
        // Counters are observational and do not affect the sequence being shrunk.
        let mut counts = coverage.borrow_mut();
        counts[0] += 1;
        counts[1] += usize::from(panel);
        counts[2] += usize::from(pending);
        counts[3] += usize::from(fault);
        drop(counts);
        if let Some(error) = &h.invariant_error {
            let directory = regression_directory();
            std::fs::create_dir_all(&directory).map_err(|e| TestCaseError::fail(e.to_string()))?;
            // Each shrinking attempt replaces this property-specific artifact.
            // Save the generated operations as well as the replay for reproduction.
            std::fs::write(
                directory.join("invariants.json"),
                h.dump()
                    .to_json()
                    .map_err(|e| TestCaseError::fail(e.to_string()))?,
            )
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
            std::fs::write(
                directory.join("invariants-case.json"),
                serde_json::to_string_pretty(&case).unwrap(),
            )
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
            return Err(TestCaseError::fail(error.to_string()));
        }
        h.finish_invariant_checks()
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let replay = ReplayDump::from_json(&h.dump().to_json().unwrap())
            .unwrap()
            .replay()
            .unwrap();
        assert_eq!(replay.core.snapshot(), h.snapshot());
        Ok(())
    });
    let coverage = coverage.into_inner();
    eprintln!(
        "coverage: {} runs; panels {}, pending profiles {}, faults {}",
        coverage[0], coverage[1], coverage[2], coverage[3]
    );
    result.unwrap();
    for count in &coverage[1..] {
        assert!(
            *count * 100 >= coverage[0] * 5,
            "state coverage below 5%: {coverage:?}"
        );
    }
}

fn regression_directory() -> std::path::PathBuf {
    std::env::var_os("TOUCHBAR_REGRESSION_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regressions")
        })
}
