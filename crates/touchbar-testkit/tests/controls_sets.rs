use touchbar_model::*;
use touchbar_sets::LayoutKind;
use touchbar_testkit::*;
#[test]
fn controls_sets_match_real_description_and_use_responsive_local_panels() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/controls");
    let plugin = plugin_dir(root).unwrap();
    let mut logic = WasmLogic::new();
    logic.register_plugin(&plugin).unwrap();
    assert_eq!(plugin.sets.len(), 3);
    let mut surface = headless(1);
    surface.height_px = 60;
    for name in ["volume", "brightness"] {
        let set = plugin.sets.iter().find(|s| s.id.as_str() == name).unwrap();
        assert_eq!(set.layout.kind, LayoutKind::Responsive);
        let variants = &set.layout.variants;
        assert_eq!(variants.len(), 2);
        assert!(
            (variants[1].minimum_width.to_px(
                &surface,
                UnitSize {
                    value: 1.0,
                    axis: Axis::Height
                }
            ) - 200.0)
                .abs()
                < 0.001
        );
        let button = &variants[0].layout;
        assert_eq!(button.hold_ms, Some(420));
        assert_eq!(button.expanded_panel, button.press_and_hold_panel);
        assert!(set.panels[0].dismiss_on_selection);
    }
}
