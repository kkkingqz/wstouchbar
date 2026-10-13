//! Informational stage-2 measurements; run with --release, no pass/fail latency threshold.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use touchbar_core::{Input, PersistKey, PersistScope};
use touchbar_logic_runtime::*;
use touchbar_model::*;
fn runtime(cache_dir: Option<PathBuf>) -> Runtime {
    Runtime::new(
        RuntimeConfig {
            cache_dir,
            limits: touchbar_logic_types::LogicLimits {
                logic_hard_ms: 10_000,
                ..Default::default()
            },
        },
        Box::<MemoryStore>::default(),
    )
    .unwrap()
}
fn id(number: u64) -> InstanceId {
    InstanceId {
        number,
        generation: Generation(1),
    }
}
fn create(r: &mut Runtime, number: u64) {
    let set: SetRef = "controls/volume".parse().unwrap();
    let key = PersistKey {
        plugin: "controls".parse().unwrap(),
        surface: "headless:benchmark".parse().unwrap(),
        scope: PersistScope::Layer(set.clone()),
    };
    assert!(matches!(
        r.create(
            Timestamp(0),
            id(number),
            &"controls".parse().unwrap(),
            "benchmark",
            &set,
            &key
        ),
        Input::LogicDescribed { .. }
    ));
}
fn call(r: &mut Runtime, number: u64, now: u64, invocation: LogicCall) {
    let response = r.call(Timestamp(now), id(number), CallId(now), &invocation);
    assert!(
        matches!(response.input, Some(Input::LogicUpdate { .. })),
        "{:?}",
        response.input
    );
}
fn load(r: &mut Runtime, bytes: &[u8]) -> f64 {
    let start = Instant::now();
    r.load_module("controls".parse().unwrap(), "benchmark", bytes)
        .unwrap();
    start.elapsed().as_secs_f64() * 1000.0
}
fn stats(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(f64::total_cmp);
    serde_json::json!({"samples": samples.len(), "median_ms": samples[samples.len()/2], "p99_ms": samples[(samples.len()*99).div_ceil(100)-1]})
}
fn main() {
    let bytes = std::fs::read(touchbar_wasm_fixtures::CONTROLS).unwrap();
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let cache = target.join(format!("controls-benchmark-cache-{}", std::process::id()));
    let mut cold = runtime(None);
    let cold_ms = load(&mut cold, &bytes);
    drop(cold);
    let mut writer = runtime(Some(cache.clone()));
    let cache_miss_ms = load(&mut writer, &bytes);
    // Allow Wasmtime's asynchronous cache writer to publish the compiled entry.
    std::thread::sleep(Duration::from_millis(250));
    drop(writer);
    let mut warm = runtime(Some(cache.clone()));
    let cached_ms = load(&mut warm, &bytes);
    let start = Instant::now();
    create(&mut warm, 1);
    let first_create_ms = start.elapsed().as_secs_f64() * 1000.0;
    warm.dispose(id(1));
    let mut creation = vec![];
    for n in 2..102 {
        let start = Instant::now();
        create(&mut warm, n);
        creation.push(start.elapsed().as_secs_f64() * 1000.0);
        warm.dispose(id(n));
    }
    create(&mut warm, 102);
    call(
        &mut warm,
        102,
        0,
        LogicCall::Init {
            config: "volume".into(),
            persisted: vec![],
        },
    );
    let mut handling = vec![];
    for n in 1..=1100 {
        let event = LogicEvent::SubscriptionData {
            tag: "sink".into(),
            data: SubscriptionData::Audio(AudioState {
                available: true,
                volume: (n % 101) as f64 / 100.0,
                muted: n % 7 == 0,
                name: "benchmark".into(),
            }),
        };
        let start = Instant::now();
        call(&mut warm, 102, n, LogicCall::Handle(event));
        if n > 100 {
            handling.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    warm.dispose(id(102));
    drop(warm);
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "component_bytes": bytes.len(), "compile_without_cache_ms": cold_ms,
        "compile_cache_miss_ms": cache_miss_ms, "load_cached_ms": cached_ms,
        "first_create_with_describe_ms": first_create_ms,
        "create_without_init": stats(creation), "handle_audio_after_100_warmups": stats(handling),
    })).unwrap());
    std::fs::remove_dir_all(cache).unwrap();
}
