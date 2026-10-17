use cosmic_text::{FontSystem, fontdb::Database};
use touchbar_ui::*;
#[test]
fn empty_font_database_gets_a_bundled_fallback_and_shapes_text() {
    let fonts = ensure_default_font(FontSystem::new_with_locale_and_db(
        "uk-UA".into(),
        Database::new(),
    ));
    assert_eq!(fonts.locale(), "uk-UA");
    assert!(fonts.db().faces().next().is_some());
    let mut painter = CpuPainter::new(fonts);
    let mut scene = Scene::new(Color::BLACK);
    scene.text(
        Rect::new(0.0, 0.0, 150.0, 40.0),
        "Touch Bar",
        20.0,
        Color::WHITE,
        TextAlign::Leading,
    );
    assert!(
        !painter
            .display_list(&scene, std::time::Duration::ZERO, MotionPolicy::Full)
            .unwrap()
            .resources
            .is_empty()
    );
}
