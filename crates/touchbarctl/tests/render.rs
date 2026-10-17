use touchbarctl::*;
#[test]
fn offline_render_arguments_are_strict_and_surface_is_bounded() {
    let options = RenderOptions::parse(
        [
            "system/fkeys",
            "--surface",
            "2170x60@6.2ppmm",
            "--theme",
            "light",
            "--plugins",
            "plugins",
            "-o",
            "out.png",
        ]
        .into_iter()
        .map(String::from),
    )
    .unwrap();
    assert_eq!(options.surface.width_px, 2170);
    assert_eq!(options.surface.px_per_mm, 6.2);
    for args in [
        vec!["system/fkeys"],
        vec!["system/fkeys", "--surface", "0x60", "-o", "out.png"],
        vec!["system/fkeys", "--theme", "blue", "-o", "out.png"],
        vec!["system/fkeys", "--surface", "8192x8192", "-o", "out.png"],
        vec![
            "system/fkeys",
            "--surface",
            "60x60@NaNppmm",
            "-o",
            "out.png",
        ],
        vec!["system/fkeys", "--unknown", "x", "-o", "out.png"],
    ] {
        assert!(RenderOptions::parse(args.into_iter().map(String::from)).is_err());
    }
}
#[test]
#[ignore = "requires EGL (Mesa llvmpipe)"]
fn env_gpu_offline_render_matches_s6_fkeys_golden_without_logic_or_session() {
    let options = RenderOptions::parse(
        [
            "system/fkeys",
            "--plugins",
            "../../plugins",
            "-o",
            "unused.png",
        ]
        .into_iter()
        .map(String::from),
    )
    .unwrap();
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    let mut painter = touchbar_ui::CpuPainter::new(
        cosmic_text::FontSystem::new_with_locale_and_db("en-US".into(), db),
    );
    let packet = render_frame(&options, &mut painter).unwrap();
    let output = touchbar_gl::Renderer::new(touchbar_gl::Backend::Surfaceless)
        .unwrap()
        .render(
            options.surface.id,
            (2008, 60),
            touchbar_gl::Orientation::Landscape,
            &packet.display_list,
            true,
        )
        .unwrap()
        .unwrap();
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../touchbar-gl/tests/golden/fkeys-12-2008x60-dark.png");
    let mut reader = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(golden).unwrap(),
    ))
    .read_info()
    .unwrap();
    let mut expected = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut expected).unwrap();
    let bad = expected
        .as_chunks::<4>()
        .0
        .iter()
        .zip(output.pixels.as_chunks::<4>().0.iter())
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 2))
        .count();
    assert!(
        bad as f64 / ((2008 * 60) as f64) <= 0.001,
        "{bad} mismatched pixels"
    );
    let output_path = std::env::temp_dir().join(format!(
        "wstouchbar-offline-render-{}.png",
        std::process::id()
    ));
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_touchbarctl"))
        .args(["render", "system/fkeys", "--plugins"])
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins"))
        .arg("-o")
        .arg(&output_path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let png = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&output_path).unwrap(),
    ))
    .read_info()
    .unwrap();
    assert_eq!((png.info().width, png.info().height), (2008, 60));
    std::fs::remove_file(output_path).unwrap();
}
