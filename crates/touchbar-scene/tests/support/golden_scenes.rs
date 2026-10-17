use std::{collections::BTreeMap, sync::Arc};
use touchbar_core::{InstanceStatus, PresentationScene, Scene, SceneItem};
use touchbar_model::*;
use touchbar_scene::*;
use touchbar_sets::*;
use touchbar_ui::{MotionPolicy, Theme};
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
            asset_kinds: BTreeMap::new(),
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
pub fn case(name: &str) -> (Scene, Documents) {
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

pub fn fonts() -> touchbar_ui::CpuPainter {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    touchbar_ui::CpuPainter::new(cosmic_text::FontSystem::new_with_locale_and_db(
        "en-US".into(),
        db,
    ))
}
pub fn frame(name: &str, width: u32, height: u32, theme: Theme) -> Frame {
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
            &mut fonts(),
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert!(frame.faults.is_empty(), "{name}: {:?}", frame.faults);
    frame
}
