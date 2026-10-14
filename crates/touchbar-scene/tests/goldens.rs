#[allow(dead_code)]
#[path = "support/golden_scenes.rs"]
mod fixtures;
use std::collections::BTreeMap;
use touchbar_ui::{Resource, Theme};
#[test]
fn primitive_and_hit_map_goldens_on_three_surfaces_and_two_themes() {
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
                frame.display_list.validate().unwrap();
                let expected_hits = match name {
                    "microphone-unavailable" | "faulted" => 0,
                    "fkeys-12" => 12,
                    "fkeys-13" => 13,
                    "volume-panel" => 2,
                    _ => 1,
                };
                if name.starts_with("fkeys") {
                    assert_eq!(
                        frame.layout.slots[0].rect.width, width as f32,
                        "function-key layer must fill surface"
                    );
                }
                assert_eq!(
                    frame.layout.hit_map.len(),
                    expected_hits,
                    "{name} {width}x{height}"
                );
                let resources = frame
                    .display_list
                    .resources
                    .iter()
                    .map(|(key, resource)| {
                        let value = match resource {
                            Resource::Raster { width, height, .. } => {
                                serde_json::json!({"kind":"raster","width":width,"height":height})
                            }
                            Resource::Shader { body } => {
                                serde_json::json!({"kind":"shader","body":body})
                            }
                        };
                        (key.clone(), value)
                    })
                    .collect::<BTreeMap<_, _>>();
                let projection = serde_json::json!({"clear":frame.display_list.clear,"commands":frame.display_list.commands,"resources":resources,"layout":frame.layout});
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                    "tests/golden/{name}-{width}x{height}-{scheme}.json"
                ));
                let actual = format!("{}\n", serde_json::to_string_pretty(&projection).unwrap());
                if std::env::var_os("UPDATE_GOLDEN").as_deref() == Some(std::ffi::OsStr::new("1")) {
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(&path, &actual).unwrap();
                }
                let expected = std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("missing {}; run UPDATE_GOLDEN=1", path.display()));
                assert_eq!(actual, expected, "{}", path.display());
            }
        }
    }
}
