#[allow(dead_code)]
mod support;
use std::{collections::BTreeMap, sync::Arc};
use touchbar_core::{InstanceStatus, PresentationScene, Scene, SceneItem};
use touchbar_model::*;
use touchbar_scene::*;
use touchbar_sets::*;
use touchbar_ui::{MotionPolicy, Resource, Theme};
fn prepared(name: &str, wide: bool, esc: bool) -> (Scene, Documents) {
    let plugin_name = if name == "fkeys" {
        "system"
    } else {
        "controls"
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../plugins/{plugin_name}"));
    let manifest = PluginManifest::from_toml(
        root.join("plugin.toml"),
        &std::fs::read_to_string(root.join("plugin.toml")).unwrap(),
    )
    .unwrap();
    let mut file = SetFile::from_toml(
        root.join(format!("sets/{name}.toml")),
        &std::fs::read_to_string(root.join(format!("sets/{name}.toml"))).unwrap(),
    )
    .unwrap();
    if wide {
        file.layout.width = Some(Length::Unit(600.0));
    }
    if esc {
        file.layout.keys.insert(
            0,
            KeyDefinition {
                key: SystemKey::Esc,
                id: None,
                width: Some(Length::Unit(150.0)),
            },
        );
    }
    let spec = file.to_spec(&manifest).unwrap();
    let values = [
        ("available", Scalar::Boolean(true)),
        ("volume", Scalar::Number(0.4)),
        ("volume_text", Scalar::Text("40%".into())),
        ("volume_label", Scalar::Text("VOL 40%".into())),
        ("volume_icon", Scalar::Text("volume".into())),
        ("volume_selected", Scalar::Boolean(false)),
        ("microphone_available", Scalar::Boolean(true)),
        ("microphone_muted", Scalar::Boolean(false)),
        ("microphone_icon", Scalar::Text("microphone".into())),
        ("microphone_text", Scalar::Text("MIC".into())),
        ("brightness", Scalar::Number(0.7)),
        ("brightness_text", Scalar::Text("70%".into())),
        ("brightness_label", Scalar::Text("LIGHT 70%".into())),
    ];
    let state = if name == "fkeys" {
        vec![]
    } else {
        values
            .into_iter()
            .map(|(n, v)| (n.into(), StateValue::Scalar(v)))
            .collect()
    };
    let item = SceneItem {
        instance: InstanceId {
            number: 1,
            generation: Generation(1),
        },
        slot: "slot".parse().unwrap(),
        set: spec,
        state,
        plane: if name == "fkeys" {
            ScenePlane::Layer(1)
        } else {
            ScenePlane::Profile
        },
        units: UnitSize {
            value: 1.0,
            axis: Axis::Height,
        },
        status: InstanceStatus::Visible,
        interactions: vec![],
    };
    let mut docs = Documents::default();
    docs.sets.insert(
        item.set.reference.clone(),
        Arc::new(SetDocument {
            file,
            assets: BTreeMap::new(),
        }),
    );
    (
        Scene {
            profile: None,
            items: vec![item],
            presentation: None,
        },
        docs,
    )
}
fn case(name: &str) -> (Scene, Documents) {
    let (mut scene, docs) = match name {
        "volume-wide" => prepared("volume", true, false),
        "volume-narrow" | "volume-panel" | "faulted" => prepared("volume", false, false),
        "microphone-muted" | "microphone-unavailable" => prepared("microphone", false, false),
        "brightness" => prepared("brightness", true, false),
        "fkeys-12" => prepared("fkeys", false, false),
        "fkeys-13" => prepared("fkeys", false, true),
        _ => unreachable!(),
    };
    if name == "faulted" {
        scene.items[0].status = InstanceStatus::Faulted;
    }
    for (field, value) in &mut scene.items[0].state {
        if name == "microphone-muted" {
            match field.as_str() {
                "microphone_muted" => *value = StateValue::Scalar(Scalar::Boolean(true)),
                "microphone_icon" => *value = StateValue::Scalar(Scalar::Text("muted".into())),
                "microphone_text" => *value = StateValue::Scalar(Scalar::Text("MUTED".into())),
                _ => {}
            }
        }
        if name == "microphone-unavailable" && field == "microphone_available" {
            *value = StateValue::Scalar(Scalar::Boolean(false));
        }
    }
    if name == "volume-panel" {
        scene.presentation = Some(PresentationScene {
            id: PresentationId(1),
            owner: scene.items[0].instance,
            panel: scene.items[0].set.panels[0].clone(),
            placement: Placement::Anchored,
            lifecycle: PresentationLifecycle::Persistent,
            anchor: Rect {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 60.0,
            },
            region: Rect::default(),
        });
    }
    (scene, docs)
}
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
                let surface = SurfaceInfo {
                    id: SurfaceId(1),
                    name: if height == 160 {
                        "window:golden"
                    } else {
                        "headless:golden"
                    }
                    .parse()
                    .unwrap(),
                    kind: if height == 160 {
                        SurfaceKind::Window
                    } else {
                        SurfaceKind::Headless
                    },
                    width_px: width,
                    height_px: height,
                    px_per_mm: 6.0,
                    caps: SurfaceCaps::default(),
                };
                let (mut scene, docs) = case(name);
                if let Some(panel) = &mut scene.presentation {
                    panel.anchor.width = Length::Unit(800.0 / 3.0)
                        .to_px(&surface, scene.items[0].units)
                        .round();
                    panel.anchor.height = height as f32;
                }
                let frame = SceneRenderer::default()
                    .frame(
                        &surface,
                        &scene,
                        &docs,
                        theme,
                        &mut support::fonts(),
                        Timestamp(0),
                        MotionPolicy::Full,
                    )
                    .unwrap();
                assert!(frame.faults.is_empty(), "{name}: {:?}", frame.faults);
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
