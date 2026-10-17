//! Informational stage-3 measurements; no pass/fail latency threshold.
//! Uses fixed Inter fonts and the same in-process fixture as the E2E tests.
#[path = "../tests/support/mod.rs"]
mod fixtures;
use std::{path::PathBuf, time::Instant};
use touchbar_model::*;
use touchbar_ui::{MotionPolicy, Theme};
fn summary(mut values: Vec<f64>) -> serde_json::Value {
    values.sort_by(f64::total_cmp);
    serde_json::json!({"samples":values.len(),"p50_ms":values[values.len()/2],"p99_ms":values[(values.len()*99).div_ceil(100)-1]})
}
fn sample(mut work: impl FnMut()) -> serde_json::Value {
    summary(
        (0..100)
            .map(|_| {
                let start = Instant::now();
                work();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect(),
    )
}
fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let backend = match args.as_slice() {
        [] => Some(touchbar_gl::Backend::Surfaceless),
        [flag] if flag == "--cpu-only" => None,
        [flag, path] if flag == "--render-node" => {
            Some(touchbar_gl::Backend::RenderNode(PathBuf::from(path)))
        }
        _ => anyhow::bail!("session_benchmark [--cpu-only | --render-node PATH]"),
    };
    let start = Instant::now();
    let fixture = fixtures::Fixture::new(2008, 60, false);
    let fixture_first_frame_ms = start.elapsed().as_secs_f64() * 1000.0;
    let cache = tempfile::tempdir()?;
    let load = || -> anyhow::Result<(f64, touchbar_scene::Documents)> {
        let runtime = touchbar_logic_runtime::Runtime::new(
            touchbar_logic_runtime::RuntimeConfig {
                cache_dir: Some(cache.path().into()),
                limits: touchbar_logic_types::LogicLimits {
                    logic_hard_ms: 10000,
                    ..Default::default()
                },
            },
            Box::<touchbar_logic_runtime::MemoryStore>::default(),
        )?;
        let mut runtime = runtime;
        let core = touchbar_core::Core::new(Default::default());
        let mut loader = touchbar_loader::Loader::new(Default::default());
        let start = Instant::now();
        let report = loader.accept(
            touchbar_loader::prepare(fixture.request(), runtime.compiler()),
            &core,
            &mut runtime,
        );
        anyhow::ensure!(report.accepted && report.issues.is_empty(), "{report:?}");
        Ok((
            start.elapsed().as_secs_f64() * 1000.0,
            loader.current().expect("accepted").documents.clone(),
        ))
    };
    let (catalog_cold_ms, docs) = load()?;
    let (catalog_warm_ms, _) = load()?;
    let snapshot = fixture.session.snapshot();
    let surface = &snapshot.surfaces[0];
    let mut scene = surface.scene.clone();
    let mut painter = fixtures::fonts();
    let mut renderer = touchbar_scene::SceneRenderer::default();
    renderer.frame(
        &surface.info,
        &scene,
        &docs,
        Theme::dark(),
        &mut painter,
        Timestamp(0),
        MotionPolicy::Disabled,
    )?;
    let mut index = 0;
    let full_before = renderer.cache_stats();
    let cpu_full = sample(|| {
        index += 1;
        std::hint::black_box(
            renderer
                .frame(
                    &surface.info,
                    &scene,
                    &docs,
                    if index % 2 == 1 {
                        Theme::light()
                    } else {
                        Theme::dark()
                    },
                    &mut painter,
                    Timestamp(0),
                    MotionPolicy::Disabled,
                )
                .unwrap(),
        );
    });
    let full_trees = renderer.cache_stats().trees_built - full_before.trees_built;
    let single_before = renderer.cache_stats();
    let cpu_single = sample(|| {
        index += 1;
        let item = scene
            .items
            .iter_mut()
            .find(|i| i.set.reference.id() == "volume")
            .unwrap();
        let value = &mut item
            .state
            .iter_mut()
            .find(|(name, _)| name == "volume_label")
            .unwrap()
            .1;
        *value = StateValue::Scalar(Scalar::Text(
            if index % 2 == 1 { "VOL 51%" } else { "VOL 50%" }.into(),
        ));
        std::hint::black_box(
            renderer
                .frame(
                    &surface.info,
                    &scene,
                    &docs,
                    Theme::dark(),
                    &mut painter,
                    Timestamp(0),
                    MotionPolicy::Disabled,
                )
                .unwrap(),
        );
    });
    let single_trees = renderer.cache_stats().trees_built - single_before.trees_built;
    let gl = if let Some(backend) = backend {
        let packet = &fixture.frame(0).display_list;
        let mut gpu = touchbar_gl::Renderer::new(backend)?;
        let description = gpu.backend_description().to_owned();
        gpu.render(
            SurfaceId(1),
            (2008, 60),
            touchbar_gl::Orientation::Landscape,
            packet,
            true,
        )?;
        let full = sample(|| {
            std::hint::black_box(
                gpu.render(
                    SurfaceId(1),
                    (2008, 60),
                    touchbar_gl::Orientation::Landscape,
                    packet,
                    true,
                )
                .unwrap(),
            );
        });
        Some(serde_json::json!({"backend":description,"full":full}))
    } else {
        None
    };
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:").map(str::to_owned))
        });
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"fixture_first_frame_ms":fixture_first_frame_ms,"catalog_cold_ms":catalog_cold_ms,"catalog_warm_ms":catalog_warm_ms,"cpu_full_theme_change":cpu_full,"cpu_one_instance_change":cpu_single,"full_trees_built":full_trees,"single_trees_built":single_trees,"gl":gl,"process_rss":rss})
        )?
    );
    Ok(())
}
