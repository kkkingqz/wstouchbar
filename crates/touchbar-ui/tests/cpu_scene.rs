use cosmic_text::{FontSystem, fontdb};
use std::time::Duration;
use touchbar_ui::*;
fn fonts() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    FontSystem::new_with_locale_and_db("en-US".into(), db)
}
#[test]
fn retained_tree_is_deterministic_through_rasterization_and_json() {
    let mut painter = CpuPainter::new(fonts());
    let tree = Node::button(WidgetId(1), "Volume", None, false);
    let mut ui = RetainedUi::new(tree);
    let bounds = Rect::new(0.0, 0.0, 120.0, 32.0);
    let first = ui.resolve_with_measurer(bounds, Theme::default(), &painter);
    let second = ui.resolve_with_measurer(bounds, Theme::default(), &painter);
    assert_eq!(first.scene, second.scene);
    assert_eq!(first.interactions, second.interactions);
    assert_eq!(first.inspector, second.inspector);
    let list = painter
        .display_list(&first.scene, Duration::ZERO, MotionPolicy::Full)
        .unwrap();
    let again = painter
        .display_list(&second.scene, Duration::ZERO, MotionPolicy::Full)
        .unwrap();
    assert_eq!(list, again);
    assert!(!list.resources.is_empty());
    let restored: DisplayList =
        serde_json::from_str(&serde_json::to_string(&list).unwrap()).unwrap();
    assert_eq!(list, restored);
}
#[test]
fn font_masks_can_be_recolored_without_rasterizing_again() {
    let mut text = TextEngine::new(fonts());
    let a = text
        .rasterize_mask("Volume", 120, 32, 17.25, TextAlign::Center)
        .unwrap()
        .clone();
    let b = text
        .rasterize_mask("Volume", 120, 32, 17.25, TextAlign::Center)
        .unwrap()
        .clone();
    assert_eq!(a, b);
    assert_eq!(text.cached_run_count(), 1);
    assert!(a.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    assert!(
        a.pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 0)
            .all(|p| p[..3] == [255, 255, 255])
    );
}
#[test]
fn svg_cache_keys_follow_source_contents_even_when_id_is_reused() {
    let red = r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="red"/></svg>"#;
    let blue = red.replace("red", "blue");
    let mut r = SvgRasterizer::default();
    let a = r
        .rasterize(&SvgAsset::new(1, 1, red).unwrap(), 4, 4)
        .unwrap();
    let b = r
        .rasterize(&SvgAsset::new(1, 1, blue).unwrap(), 4, 4)
        .unwrap();
    assert_ne!(a.pixels, b.pixels);
    let alias = r
        .rasterize(&SvgAsset::new(999, 45, red).unwrap(), 4, 4)
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(&a.pixels, &alias.pixels));
    assert_eq!(r.cached_count(), 2);
}
#[test]
fn unknown_symbols_use_a_visible_placeholder_and_warn_once_per_name() {
    let mut catalog = SymbolCatalog::default();
    let fallback = catalog.named_image("not-an-icon", 16).unwrap();
    assert!(fallback.pixels.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
    assert_eq!(catalog.take_warnings().len(), 1);
    catalog.named_image("not-an-icon", 24).unwrap();
    catalog.named_image("volume", 16).unwrap();
    assert!(catalog.take_warnings().is_empty());
    catalog.named_image("another-missing", 16).unwrap();
    assert_eq!(catalog.take_warnings().len(), 1);
}
#[test]
fn light_palette_preserves_roles_and_custom_accent() {
    let light = Theme::light().with_accent(Color::rgb(0.1, 0.4, 0.8));
    let dark = Theme::dark();
    assert!(light.background.red > 0.8);
    assert!(light.foreground.red < 0.2);
    assert!(dark.background.red < 0.2);
    assert_eq!(light.accent, Color::rgb(0.1, 0.4, 0.8));
}
#[test]
fn sampled_motion_and_effects_are_portable_and_have_explicit_time() {
    let program = std::sync::Arc::new(
        EffectProgram::compile("let color = mix(background, accent, 0.5 + 0.5 * sin(time));")
            .unwrap(),
    );
    let mut scene = Scene::new(Color::BLACK);
    scene.push_motion(
        Point::new(0.0, 0.0),
        Motion {
            id: MotionId(1),
            from: VisualTransform {
                translation: Point::new(10.0, 0.0),
                ..VisualTransform::IDENTITY
            },
            to: VisualTransform::IDENTITY,
            started: Duration::ZERO,
            duration: Duration::from_secs(1),
            easing: Easing::Linear,
            playback: MotionPlayback::Once,
        },
    );
    scene.shader_effect(
        ShaderEffect {
            program,
            parameters: [0.0; 8],
            opacity: 1.0,
            started: Duration::ZERO,
            period: Some(Duration::from_secs(2)),
        },
        Rect::new(0.0, 0.0, 32.0, 16.0),
        Theme::default(),
    );
    scene.pop_motion();
    let list =
        DisplayList::from_scene(&scene, Duration::from_millis(500), MotionPolicy::Full).unwrap();
    assert!(
        matches!(list.commands[0],DrawCommand::PushTransform {transform,..} if transform.translation.x==5.0)
    );
    assert!(
        matches!(&list.commands[1],DrawCommand::ShaderEffect {uniforms,..} if uniforms.time==0.5)
    );
    let reduced =
        DisplayList::from_scene(&scene, Duration::from_millis(500), MotionPolicy::Reduced).unwrap();
    assert!(
        matches!(&reduced.commands[1],DrawCommand::ShaderEffect {uniforms,..} if uniforms.time==0.0)
    );
    let restored: DisplayList =
        serde_json::from_slice(&serde_json::to_vec(&list).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(list, restored);
}

#[test]
fn text_raster_budget_and_invalid_font_sizes_are_rejected_before_allocation() {
    let mut engine = TextEngine::new(fonts());
    assert!(
        engine
            .rasterize_mask("x", 8193, 1, 16.0, TextAlign::Leading)
            .is_err()
    );
    assert!(
        engine
            .rasterize_mask("x", 8192, 513, 16.0, TextAlign::Leading)
            .is_err()
    );
    assert!(
        engine
            .rasterize_mask("x", 1, 1, f32::NAN, TextAlign::Leading)
            .is_err()
    );
    assert!(
        engine
            .rasterize_mask("x", 1, 1, -1.0, TextAlign::Leading)
            .is_err()
    );
}
