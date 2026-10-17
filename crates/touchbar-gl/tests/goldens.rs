#[allow(dead_code)]
#[path = "../../touchbar-scene/tests/support/golden_scenes.rs"]
mod fixtures;
use touchbar_gl::*;
use touchbar_model::SurfaceId;
use touchbar_ui::Theme;
fn decode(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()))
        .read_info()
        .unwrap();
    let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut bytes).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    (info.width, info.height, bytes)
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_png_goldens_from_all_fifty_four_cpu_frames() {
    let mut renderer = Renderer::new(Backend::Surfaceless).unwrap();
    // Driver rounding may differ by two channel values; at most 0.1% pixels may exceed it.
    const CHANNEL_TOLERANCE: u8 = 2;
    const PIXEL_FRACTION: f64 = 0.001;
    for name in [
        "volume-narrow",
        "volume-wide",
        "volume-panel",
        "microphone-muted",
        "microphone-unavailable",
        "brightness",
        "faulted",
        "fkeys-12",
        "fkeys-13",
    ] {
        for (width, height) in [(2008, 60), (2170, 60), (1200, 160)] {
            for (scheme, theme) in [("dark", Theme::dark()), ("light", Theme::light())] {
                let frame = fixtures::frame(name, width, height, theme);
                let output = renderer
                    .render(
                        SurfaceId(1),
                        (width, height),
                        Orientation::Landscape,
                        &frame.display_list,
                        true,
                    )
                    .unwrap()
                    .unwrap();
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/golden/{name}-{width}x{height}-{scheme}.png"));
                if std::env::var_os("UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    output.save_png(&path).unwrap();
                }
                assert!(
                    path.exists(),
                    "missing {}; run UPDATE_GOLDEN=1 cargo xtask test-env gpu",
                    path.display()
                );
                let (w, h, expected) = decode(&path);
                assert_eq!((w, h), (width, height));
                assert_eq!(expected.len(), output.pixels.len());
                let bad = expected
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(output.pixels.as_chunks::<4>().0.iter())
                    .filter(|(a, b)| {
                        a.iter()
                            .zip(b.iter())
                            .any(|(a, b)| a.abs_diff(*b) > CHANNEL_TOLERANCE)
                    })
                    .count();
                if bad as f64 / (u64::from(w) * u64::from(h)) as f64 > PIXEL_FRACTION {
                    let root = std::env::var_os("CARGO_TARGET_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| {
                            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
                        });
                    let root = root.join("golden-diff");
                    std::fs::create_dir_all(&root).unwrap();
                    let stem = path.file_stem().unwrap().to_string_lossy();
                    output
                        .save_png(root.join(format!("{stem}-actual.png")))
                        .unwrap();
                    let diff = expected
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(output.pixels.as_chunks::<4>().0.iter())
                        .flat_map(|(a, b)| {
                            let delta = a
                                .iter()
                                .zip(b)
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap()
                                .saturating_mul(8);
                            [delta, 0, 0, 255]
                        })
                        .collect();
                    RenderedFrame {
                        width: w,
                        height: h,
                        pixels: diff,
                        damage: vec![],
                    }
                    .save_png(root.join(format!("{stem}-diff.png")))
                    .unwrap();
                    panic!("{}: {bad} differing pixels", path.display());
                }
            }
        }
    }
}
