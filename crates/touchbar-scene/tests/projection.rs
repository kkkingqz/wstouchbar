use std::{collections::BTreeMap, sync::Arc};
use touchbar_core::{InstanceStatus, Scene, SceneItem, ScrubberState, WidgetInteraction};
use touchbar_model::*;
use touchbar_scene::*;
use touchbar_sets::{PluginManifest, SetFile};
use touchbar_ui::{CpuPainter, MotionPolicy, Theme};
fn surface() -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(1),
        name: "headless:test".parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 2008,
        height_px: 60,
        px_per_mm: 6.0,
        caps: SurfaceCaps::default(),
    }
}
fn painter() -> CpuPainter {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../assets/fonts/test/InterVariable.ttf").to_vec());
    db.set_sans_serif_family("Inter Variable");
    CpuPainter::new(cosmic_text::FontSystem::new_with_locale_and_db(
        "en-US".into(),
        db,
    ))
}
fn fixture(layout: &str, state: StatePatch) -> (Scene, Documents) {
    let file = SetFile::from_toml(
        "test.toml",
        &format!("version=1\nid='test'\nlabel='Test'\n{layout}"),
    )
    .unwrap();
    let plugin = PluginManifest::from_toml(
        "plugin.toml",
        "id='p'\nversion='1'\napi='1'\nname='P'\ndescription='P'\nlogic='logic.wasm'",
    )
    .unwrap();
    let spec = file.to_spec(&plugin).unwrap();
    let mut docs = Documents::default();
    docs.sets.insert(
        spec.reference.clone(),
        Arc::new(SetDocument {
            file,
            assets: BTreeMap::new(),
            asset_kinds: BTreeMap::new(),
        }),
    );
    let item = SceneItem {
        instance: InstanceId {
            number: 1,
            generation: Generation(1),
        },
        slot: "slot".parse().unwrap(),
        set: spec,
        state,
        plane: ScenePlane::Profile,
        units: UnitSize::resolve(&surface(), None, None),
        status: InstanceStatus::Visible,
        interactions: vec![],
    };
    (
        Scene {
            profile: None,
            items: vec![item],
            presentation: None,
        },
        docs,
    )
}
fn draw(scene: &Scene, docs: &Documents) -> Frame {
    SceneRenderer::default()
        .frame(
            &surface(),
            scene,
            docs,
            Theme::dark(),
            &mut painter(),
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap()
}
fn scalar(name: &str, value: Scalar) -> (String, StateValue) {
    (name.into(), StateValue::Scalar(value))
}
#[test]
fn maps_logical_auto_ids_to_physical_button_bounds() {
    let (scene, docs) = fixture("[layout]\nkind='button'\nlabel='Go'", vec![]);
    let frame = draw(&scene, &docs);
    frame.display_list.validate().unwrap();
    assert_eq!(frame.layout.hit_map.len(), 1);
    let hit = &frame.layout.hit_map[0];
    assert_eq!(hit.widget, scene.items[0].set.widgets[0].id);
    assert_eq!(
        hit.rect,
        Rect {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 60.0
        }
    );
    assert_eq!(frame.layout.slots[0].rect, hit.rect);
}
#[test]
fn invisible_child_is_removed_from_layout_and_disabled_child_from_hits() {
    let (scene, docs) = fixture(
        "[layout]\nkind='row'\nchildren=[{kind='button',id='hide',visible_from='show',width=100},{kind='button',id='keep',enabled_from='enabled',width=100}]",
        vec![
            scalar("show", Scalar::Boolean(false)),
            scalar("enabled", Scalar::Boolean(true)),
        ],
    );
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].rect.x, 0.0);
    let mut disabled = scene.clone();
    disabled.items[0].state[1] = scalar("enabled", Scalar::Boolean(false));
    assert!(draw(&disabled, &docs).layout.hit_map.is_empty());
}
#[test]
fn repeat_keys_follow_records_after_reorder() {
    let records = |keys: &[&str]| {
        StateValue::Records(
            keys.iter()
                .map(|key| BTreeMap::from([("id".into(), Scalar::Text((*key).into()))]))
                .collect(),
        )
    };
    let (mut scene, docs) = fixture(
        "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='button',id='cell',label_from='record.id',width=100}",
        vec![("rows".into(), records(&["a", "b"]))],
    );
    let frame = draw(&scene, &docs);
    assert_eq!(
        frame
            .layout
            .hit_map
            .iter()
            .map(|hit| hit.record_key.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    scene.items[0].state[0].1 = records(&["b", "a"]);
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.hit_map[0].record_key.as_deref(), Some("b"));
    assert_eq!(frame.layout.hit_map[0].widget.as_str(), "cell");
}
#[test]
fn scrubber_exports_unscrolled_geometry_and_paints_host_offset() {
    let records = StateValue::Records(
        ["a", "b", "c"]
            .iter()
            .map(|key| BTreeMap::from([("id".into(), Scalar::Text((*key).into()))]))
            .collect(),
    );
    let (mut scene, docs) = fixture(
        "[layout]\nkind='scrubber'\nid='list'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.id',width=200}",
        vec![("rows".into(), records)],
    );
    let before = draw(&scene, &docs);
    assert_eq!(before.layout.scrubbers.len(), 1);
    let geometry = &before.layout.scrubbers[0];
    assert_eq!(geometry.content_width, 360.0);
    assert_eq!(geometry.items[2].rect.x, 240.0);
    assert_eq!(before.layout.hit_map.len(), 1);
    scene.items[0].interactions.push(WidgetInteraction {
        widget: "list".parse().unwrap(),
        record_key: None,
        pressed: true,
        value: None,
        scrubber: ScrubberState {
            offset_px: 100.0,
            highlighted: Some("b".into()),
            selected: Some("b".into()),
        },
    });
    let after = draw(&scene, &docs);
    assert_eq!(after.layout.scrubbers, before.layout.scrubbers);
    assert_ne!(after.display_list, before.display_list);
}
#[test]
fn only_top_layer_draws_and_accepts_hits() {
    let (mut scene, docs) = fixture("[layout]\nkind='button'\nlabel='Go'", vec![]);
    let mut lower = scene.items[0].clone();
    lower.instance.number = 2;
    lower.plane = ScenePlane::Layer(1);
    let mut upper = lower.clone();
    upper.instance.number = 3;
    upper.plane = ScenePlane::Layer(2);
    scene.items.extend([lower, upper]);
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].instance.number, 3);
    assert_eq!(frame.layout.slots.len(), 3);
}
#[test]
fn loading_faulted_and_hidden_do_not_expose_hits() {
    let (mut scene, docs) = fixture("[layout]\nkind='button'\nlabel='Go'", vec![]);
    let mut lengths = vec![];
    for status in [
        InstanceStatus::Loading,
        InstanceStatus::Faulted,
        InstanceStatus::Hidden,
    ] {
        scene.items[0].status = status;
        let frame = draw(&scene, &docs);
        assert!(frame.layout.hit_map.is_empty());
        lengths.push(frame.display_list.commands.len());
    }
    assert!(lengths[1] > lengths[2]);
}
#[test]
fn malformed_state_is_local_fault_and_preserves_other_instances() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nlabel_from='label'",
        vec![scalar("label", Scalar::Number(1.0))],
    );
    let mut healthy = scene.items[0].clone();
    healthy.instance.number = 2;
    healthy.slot = "other".parse().unwrap();
    healthy.state = vec![scalar("label", Scalar::Text("Good".into()))];
    scene.items.push(healthy);
    let frame = draw(&scene, &docs);
    assert_eq!(frame.faults.len(), 1);
    assert_eq!(frame.faults[0].0.number, 1);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].instance.number, 2);
}
#[test]
fn responsive_and_host_slider_override_use_real_geometry() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='responsive'\nwidth=400\nvariants=[{representation='minimal',minimum_width=0,layout={kind='button',id='small',label='Small'}},{representation='full',minimum_width=333,layout={kind='slider',id='level',value_from='level'}}]",
        vec![scalar("level", Scalar::Number(0.1))],
    );
    let before = draw(&scene, &docs);
    assert_eq!(before.layout.representations[0].1, "full");
    scene.items[0].interactions.push(WidgetInteraction {
        widget: "level".parse().unwrap(),
        record_key: None,
        pressed: true,
        value: Some(0.9),
        scrubber: ScrubberState::default(),
    });
    let after = draw(&scene, &docs);
    assert_eq!(after.layout.hit_map, before.layout.hit_map);
    assert_ne!(after.display_list, before.display_list);
}
#[test]
fn key_row_has_all_key_labels_and_distinct_generated_ids() {
    let (scene, docs) = fixture(
        "[layout]\nkind='key-row'\nwidth='100%w'\nkeys=[{key='esc'},{key='f1'},{key='f2'}]",
        vec![],
    );
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.hit_map.len(), 3);
    let ids = frame
        .layout
        .hit_map
        .iter()
        .map(|hit| hit.widget.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 3);
    assert_eq!(frame.layout.hit_map[0].rect.width, 669.0);
}
#[test]
fn canvas_and_time_sampled_motion_are_deterministic() {
    let paint = CanvasPaint {
        color: CanvasColor::Role(ColorRole::Accent),
        opacity: 1.0,
    };
    let (scene, docs) = fixture(
        "[layout]\nkind='canvas'\ncanvas_from='drawing'\nmotion={id=1,duration_ms=1000,easing='linear',playback='once',from={opacity=0},to={opacity=1}}",
        vec![(
            "drawing".into(),
            StateValue::Canvas(vec![CanvasCommand::FillRect {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 100.0,
                    height: 100.0,
                },
                radius: 0.0,
                paint,
            }]),
        )],
    );
    let mut renderer = SceneRenderer::default();
    let mut fonts = painter();
    let first = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    let middle = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(500),
            MotionPolicy::Full,
        )
        .unwrap();
    assert!(middle.animate);
    assert_ne!(first.display_list, middle.display_list);
    let same = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(500),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(same.display_list, middle.display_list);
    assert!(
        middle
            .display_list
            .commands
            .iter()
            .any(|c| matches!(c, touchbar_ui::DrawCommand::RoundedRect { .. }))
    );
}
#[test]
fn shaders_images_and_all_style_bindings_are_materialized() {
    let (scene, mut docs) = fixture(
        "[layout]\nkind='row'\nchildren=[{kind='button',id='styled',label_from='label',color_from='fg',background_from='bg',pressed_background_from='pressed',pressed_from='down',selected_from='selected',width=100},{kind='image',asset_from='image',width=40},{kind='shader-effect',width=100,effect={id=1,source='let color = vec4<f32>(0.2, 0.3, 0.4, 1.0);',parameters=['amount'],period_ms=100}}]",
        vec![
            scalar("label", Scalar::Text("Go".into())),
            scalar("fg", Scalar::Text("foreground".into())),
            scalar("bg", Scalar::Text("accent".into())),
            scalar("pressed", Scalar::Text("destructive".into())),
            scalar("down", Scalar::Boolean(true)),
            scalar("selected", Scalar::Boolean(false)),
            scalar("image", Scalar::Text("ready".into())),
            scalar("amount", Scalar::Number(0.4)),
        ],
    );
    let doc = Arc::make_mut(docs.sets.values_mut().next().unwrap());
    doc.assets.insert(
        "ready".into(),
        touchbar_ui::Image::rgba8(1, 1, 1, 1, vec![255; 4]).unwrap(),
    );
    let frame = draw(&scene, &docs);
    assert!(frame.faults.is_empty(), "{:?}", frame.faults);
    assert!(
        frame
            .display_list
            .commands
            .iter()
            .any(|c| matches!(c, touchbar_ui::DrawCommand::ShaderEffect { .. }))
    );
    assert!(
        frame
            .display_list
            .resources
            .values()
            .any(|r| matches!(r, touchbar_ui::Resource::Raster { width: 1, .. }))
    );
}

fn composed_shaders(count: usize, unique: bool, panel: bool) -> (Scene, Documents) {
    let mut items = Vec::new();
    let mut documents = Documents::default();
    for index in 0..count {
        let source = format!(
            "let color = vec4<f32>(0.{}, 0.3, 0.4, 1.0);",
            if unique { index + 1 } else { 1 }
        );
        let mut layout =
            format!("[layout]\nkind='shader-effect'\neffect={{id=1,source='{source}'}}");
        if panel && index == 0 {
            layout.push_str(&format!("\n[[panels]]\nid='extra'\n[panels.layout]\nkind='shader-effect'\neffect={{id=2,source='{source}'}}"));
        }
        let (mut scene, mut docs) = fixture(&layout, vec![]);
        let mut item = scene.items.remove(0);
        let mut document = docs.sets.remove(&item.set.reference).unwrap();
        let id = format!("shader{index}");
        Arc::make_mut(&mut document).file.id = id.clone();
        item.set.reference = format!("p/{id}").parse().unwrap();
        item.instance.number = index as u64 + 1;
        item.slot = format!("slot{index}").parse().unwrap();
        documents.sets.insert(item.set.reference.clone(), document);
        items.push(item);
    }
    let presentation = panel.then(|| touchbar_core::PresentationScene {
        id: PresentationId(1),
        owner: items[0].instance,
        panel: items[0].set.panels[0].clone(),
        placement: Placement::Region,
        lifecycle: PresentationLifecycle::Transient,
        anchor: Rect::default(),
        region: Rect::default(),
    });
    (
        Scene {
            profile: None,
            items,
            presentation,
        },
        documents,
    )
}

fn assert_composed_shaders(count: usize, unique: bool, panel: bool) {
    let (scene, docs) = composed_shaders(count, unique, panel);
    let frame = draw(&scene, &docs);
    assert!(frame.faults.is_empty(), "{:?}", frame.faults);
    assert_eq!(
        frame
            .display_list
            .commands
            .iter()
            .filter(|c| matches!(c, touchbar_ui::DrawCommand::ShaderEffect { .. }))
            .count(),
        count + usize::from(panel)
    );
    frame.display_list.validate().unwrap();
}

#[test]
fn shader_node_quota_is_per_instance_not_composed_surface() {
    assert_composed_shaders(5, false, false);
}
#[test]
fn shader_program_quota_is_per_instance_not_composed_surface() {
    assert_composed_shaders(9, true, false);
}
#[test]
fn panel_shader_can_compose_with_four_valid_instances() {
    assert_composed_shaders(4, false, true);
}
#[test]
fn geometry_cache_reacts_to_surface_percentages_even_when_slot_width_is_fixed() {
    let (scene, docs) = fixture(
        "[layout]\nkind='row'\nchildren=[{kind='button',id='a',width='5%w'},{kind='button',id='b',width=30}]",
        vec![],
    );
    let mut renderer = SceneRenderer::default();
    let mut fonts = painter();
    let mut surface = surface();
    let first = renderer
        .frame(
            &surface,
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    let counts = renderer.cache_stats();
    renderer
        .frame(
            &surface,
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(100),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(renderer.cache_stats(), counts);
    surface.width_px = 1200;
    let changed = renderer
        .frame(
            &surface,
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(100),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_ne!(
        first.layout.hit_map[0].rect.width,
        changed.layout.hit_map[0].rect.width
    );
    assert_eq!(changed.layout.hit_map[0].rect.width, 60.0);
}
#[test]
fn panel_region_uses_owner_slot_without_requiring_anchor() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nid='base'\nlabel='Base'\n[[panels]]\nid='controls'\n[panels.layout]\nkind='slider'\nid='panel-level'\nvalue=0.5",
        vec![],
    );
    scene.presentation = Some(touchbar_core::PresentationScene {
        id: PresentationId(1),
        owner: scene.items[0].instance,
        panel: scene.items[0].set.panels[0].clone(),
        placement: Placement::Region,
        lifecycle: PresentationLifecycle::Transient,
        anchor: Rect::default(),
        region: Rect::default(),
    });
    let frame = draw(&scene, &docs);
    assert_eq!(
        frame.layout.presentation.unwrap().bounds,
        frame.layout.slots[0].rect
    );
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].plane, ScenePlane::Presentation);
}
#[test]
fn full_panel_covers_all_base_hits_and_scrubbers() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nid='base'\nlabel='Base'\n[[panels]]\nid='controls'\n[panels.layout]\nkind='slider'\nid='panel-level'\nvalue=0.5",
        vec![],
    );
    scene.presentation = Some(touchbar_core::PresentationScene {
        id: PresentationId(1),
        owner: scene.items[0].instance,
        panel: scene.items[0].set.panels[0].clone(),
        placement: Placement::Full,
        lifecycle: PresentationLifecycle::Transient,
        anchor: Rect::default(),
        region: Rect::default(),
    });
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.presentation.unwrap().bounds.width, 2008.0);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].plane, ScenePlane::Presentation);
}
#[test]
fn canvas_paths_flatten_curves_and_triangulate_concave_polygon() {
    let paint = CanvasPaint {
        color: CanvasColor::Role(ColorRole::Foreground),
        opacity: 1.0,
    };
    let p = |x, y| Point { x, y };
    let (scene, docs) = fixture(
        "[layout]\nkind='canvas'\ncanvas_from='drawing'",
        vec![(
            "drawing".into(),
            StateValue::Canvas(vec![
                CanvasCommand::StrokePath {
                    segments: vec![
                        CanvasPathSegment::MoveTo(p(0.0, 0.0)),
                        CanvasPathSegment::QuadraticTo {
                            control: p(20.0, 30.0),
                            endpoint: p(50.0, 0.0),
                        },
                        CanvasPathSegment::CubicTo {
                            control_one: p(60.0, 0.0),
                            control_two: p(70.0, 20.0),
                            endpoint: p(80.0, 10.0),
                        },
                    ],
                    width: 2.0,
                    paint,
                },
                CanvasCommand::FillPath {
                    segments: vec![
                        CanvasPathSegment::MoveTo(p(0.0, 0.0)),
                        CanvasPathSegment::LineTo(p(20.0, 0.0)),
                        CanvasPathSegment::LineTo(p(10.0, 10.0)),
                        CanvasPathSegment::LineTo(p(20.0, 20.0)),
                        CanvasPathSegment::LineTo(p(0.0, 20.0)),
                        CanvasPathSegment::Close,
                    ],
                    paint,
                },
            ]),
        )],
    );
    let frame = draw(&scene, &docs);
    assert!(frame.faults.is_empty(), "{:?}", frame.faults);
    assert!(frame.display_list.commands.iter().any(
        |c| matches!(c,touchbar_ui::DrawCommand::TriangleMesh{triangles,..} if triangles.len()==9)
    ));
    assert!(
        frame
            .display_list
            .commands
            .iter()
            .filter(|c| matches!(c, touchbar_ui::DrawCommand::Line { .. }))
            .count()
            >= 20
    );
}
#[test]
fn text_and_canvas_expansion_budgets_fault_only_the_instance() {
    for commands in [
        vec![CanvasCommand::Polyline {
            points: vec![Point { x: 0.0, y: 0.0 }; 2049],
            width: 1.0,
            paint: CanvasPaint {
                color: CanvasColor::Role(ColorRole::Accent),
                opacity: 1.0,
            },
        }],
        vec![CanvasCommand::Text {
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
            text: "Huge".into(),
            size: 100_000.0,
            paint: CanvasPaint {
                color: CanvasColor::Role(ColorRole::Accent),
                opacity: 1.0,
            },
            align: TextAlign::Center,
        }],
    ] {
        let (scene, docs) = fixture(
            "[layout]\nkind='canvas'\ncanvas_from='drawing'",
            vec![("drawing".into(), StateValue::Canvas(commands))],
        );
        let frame = draw(&scene, &docs);
        assert_eq!(frame.faults.len(), 1);
    }
}
#[test]
fn instance_tree_cache_changes_without_recomposing_unchanged_slots() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nid='go'\nlabel_from='label'",
        vec![scalar("label", Scalar::Text("One".into()))],
    );
    let mut renderer = SceneRenderer::default();
    let mut fonts = painter();
    renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    let first = renderer.cache_stats();
    scene.items[0].state[0] = scalar("label", Scalar::Text("Two".into()));
    renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(renderer.cache_stats().trees_built, first.trees_built + 1);
    assert_eq!(
        renderer.cache_stats().layouts_resolved,
        first.layouts_resolved
    );
    scene.items[0].interactions.push(WidgetInteraction {
        widget: "go".parse().unwrap(),
        record_key: None,
        pressed: true,
        value: None,
        scrubber: ScrubberState::default(),
    });
    renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(renderer.cache_stats().trees_built, first.trees_built + 2);
    renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::light(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(renderer.cache_stats().trees_built, first.trees_built + 3);
    assert_eq!(
        renderer.cache_stats().layouts_resolved,
        first.layouts_resolved
    );
}
#[test]
fn repeated_visual_wrappers_count_toward_expanded_node_budget() {
    let records = StateValue::Records(
        (0..300)
            .map(|n| BTreeMap::from([("id".into(), Scalar::Text(n.to_string()))]))
            .collect(),
    );
    let (scene, docs) = fixture(
        "[layout]\nkind='repeat'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='button',label_from='record.id'}",
        vec![("rows".into(), records)],
    );
    let frame = draw(&scene, &docs);
    assert_eq!(frame.faults.len(), 1);
    assert!(frame.faults[0].1.contains("node budget"));
}
#[test]
fn root_visibility_excludes_slot_from_width_distribution() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nid='go'\nlabel='Go'\nvisible_from='show'",
        vec![scalar("show", Scalar::Boolean(false))],
    );
    let mut second = scene.items[0].clone();
    second.instance.number = 2;
    second.slot = "next".parse().unwrap();
    second.state[0] = scalar("show", Scalar::Boolean(true));
    scene.items.push(second);
    let frame = draw(&scene, &docs);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].rect.x, 0.0);
}
#[test]
fn profile_groups_preserve_empty_slot_bounds_and_profile_units_for_spaces() {
    use touchbar_profiles::{ProfileDocument, resolve_profiles};
    let (mut scene, mut docs) = fixture("[layout]\nkind='button'\nid='go'\nlabel='Go'", vec![]);
    let reference: ProfileRef = "p/default".parse().unwrap();
    let document=ProfileDocument::from_toml("profile.toml","version=1\nid='default'\ndefault=true\n[units]\nu='2%h'\n[[element]]\nkind='group'\nid='pair'\nlayout='equal-width'\nspacing=10\nelement=[{kind='slot',id='slot',items=['p/test']},{kind='slot',id='empty',items=['p/test']}]\n[[element]]\nkind='flexible-space'\nweight=1").unwrap();
    docs.profiles = resolve_profiles(&BTreeMap::from([(reference.clone(), document)])).unwrap();
    scene.profile = Some(reference);
    // The live instance's already-resolved units are authoritative; empty slots use profile units.
    scene.items[0].units = UnitSize {
        value: 2.0,
        axis: Axis::Height,
    };
    let frame = draw(&scene, &docs);
    let empty = frame
        .layout
        .named_slots
        .iter()
        .find(|s| s.slot.as_str() == "empty")
        .unwrap();
    assert!(empty.instance.is_none());
    assert_eq!(empty.bounds.unwrap().width, 320.0);
    assert_eq!(empty.bounds.unwrap().x, 332.0);
    assert_eq!(frame.layout.slots.len(), 1);
    assert_eq!(frame.layout.slots[0].rect.width, 320.0);
}
#[test]
fn changed_state_preserves_unchanged_motion_timeline_and_layout_geometry() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='button'\nid='go'\nlabel_from='label'\nmotion={id=1,duration_ms=1000,easing='linear',playback='once',from={opacity=0},to={opacity=1}}",
        vec![scalar("label", Scalar::Text("One".into()))],
    );
    let mut renderer = SceneRenderer::default();
    let mut fonts = painter();
    let initial = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    scene.items[0].state[0] = scalar("label", Scalar::Text("Two".into()));
    let changed = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(500),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(initial.layout.hit_map, changed.layout.hit_map);
    assert!(changed.display_list.commands.iter().any(|c|matches!(c,touchbar_ui::DrawCommand::PushTransform{transform,..} if transform.opacity==0.5)));
    let counts = renderer.cache_stats();
    renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(750),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(renderer.cache_stats(), counts);
}
#[test]
fn symbol_rasters_and_unknown_warning_survive_tree_rebuild() {
    let (mut scene, docs) = fixture(
        "[layout]\nkind='icon'\nsymbol_from='symbol'",
        vec![
            scalar("symbol", Scalar::Text("missing-symbol".into())),
            scalar("revision", Scalar::Number(1.0)),
        ],
    );
    let mut renderer = SceneRenderer::default();
    let mut fonts = painter();
    let first = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert_eq!(first.warnings.len(), 1);
    scene.items[0].state[1] = scalar("revision", Scalar::Number(2.0));
    let second = renderer
        .frame(
            &surface(),
            &scene,
            &docs,
            Theme::dark(),
            &mut fonts,
            Timestamp(0),
            MotionPolicy::Full,
        )
        .unwrap();
    assert!(second.warnings.is_empty());
    let first = first
        .display_list
        .resources
        .values()
        .find_map(|r| {
            if let touchbar_ui::Resource::Raster { pixels, .. } = r {
                Some(pixels)
            } else {
                None
            }
        })
        .unwrap();
    let second = second
        .display_list
        .resources
        .values()
        .find_map(|r| {
            if let touchbar_ui::Resource::Raster { pixels, .. } = r {
                Some(pixels)
            } else {
                None
            }
        })
        .unwrap();
    assert!(Arc::ptr_eq(first, second));
}
#[test]
fn scrubber_cell_geometry_rounds_padding_and_spacing_once() {
    let records = StateValue::Records(
        ["a", "b"]
            .iter()
            .map(|key| BTreeMap::from([("id".into(), Scalar::Text((*key).into()))]))
            .collect(),
    );
    let (scene, docs) = fixture(
        "[layout]\nkind='scrubber'\nid='list'\nrepeat_from='rows'\nrepeat_key='id'\npadding=1\nspacing=1\ntemplate={kind='text',text_from='record.id',width=40}",
        vec![("rows".into(), records)],
    );
    let frame = draw(&scene, &docs);
    let geometry = &frame.layout.scrubbers[0];
    assert_eq!(geometry.items[0].rect.x, 1.0);
    assert_eq!(geometry.items[1].rect.x, 26.0);
    assert_eq!(geometry.content_width, 51.0);
}
#[test]
fn zero_width_scrubber_cells_are_not_selectable() {
    let records = StateValue::Records(vec![BTreeMap::from([(
        "id".into(),
        Scalar::Text("a".into()),
    )])]);
    let (scene, docs) = fixture(
        "[layout]\nkind='scrubber'\nid='list'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.id',width=0}",
        vec![("rows".into(), records)],
    );
    let frame = draw(&scene, &docs);
    assert!(frame.layout.scrubbers[0].items.is_empty());
    assert_eq!(frame.layout.scrubbers[0].content_width, 0.0);
}
#[test]
fn raster_budget_failure_preserves_other_instance_frame() {
    let records = StateValue::Records(vec![BTreeMap::from([(
        "id".into(),
        Scalar::Text("a".into()),
    )])]);
    let (mut scene, mut docs) = fixture(
        "[layout]\nkind='scrubber'\nid='list'\nrepeat_from='rows'\nrepeat_key='id'\ntemplate={kind='text',text_from='record.id',width='100%w'}",
        vec![("rows".into(), records)],
    );
    scene.items[0].units = UnitSize {
        value: 100.0,
        axis: Axis::Height,
    };
    // The inner cell exceeds the raster edge budget while its outer viewport stays small.
    Arc::make_mut(docs.sets.values_mut().next().unwrap())
        .file
        .layout
        .template
        .as_mut()
        .unwrap()
        .width = Some(Length::Unit(200.0));
    Arc::make_mut(docs.sets.values_mut().next().unwrap())
        .file
        .layout
        .template
        .as_mut()
        .unwrap()
        .font_size = Some(Length::Unit(1.0));
    let (mut other, other_docs) =
        fixture("[layout]\nkind='button'\nid='good'\nlabel='Good'", vec![]);
    let mut doc = (**other_docs.sets.values().next().unwrap()).clone();
    doc.file.id = "good".into();
    let manifest = PluginManifest::from_toml(
        "plugin.toml",
        "id='p'\nversion='1'\napi='1'\nname='P'\ndescription='P'\nlogic='logic.wasm'",
    )
    .unwrap();
    other.items[0].set = doc.file.to_spec(&manifest).unwrap();
    other.items[0].instance.number = 2;
    other.items[0].slot = "good".parse().unwrap();
    docs.sets
        .insert(other.items[0].set.reference.clone(), Arc::new(doc));
    scene.items.extend(other.items);
    // Root basis must fit while this test exercises the inner cell's width.
    Arc::make_mut(docs.sets.get_mut(&"p/test".parse().unwrap()).unwrap())
        .file
        .layout
        .width = Some(Length::Percent {
        value: 20.0,
        axis: Axis::Width,
    });
    let frame = draw(&scene, &docs);
    assert_eq!(frame.faults.len(), 1);
    assert_eq!(frame.faults[0].0.number, 1);
    assert_eq!(frame.layout.hit_map.len(), 1);
    assert_eq!(frame.layout.hit_map[0].widget.as_str(), "good");
}
#[test]
fn manifest_png_preserves_color_while_symbolic_svg_uses_foreground_mask() {
    let (scene, mut docs) = fixture("[layout]\nkind='image'\nasset='picture'\nwidth=100", vec![]);
    let doc = Arc::make_mut(docs.sets.values_mut().next().unwrap());
    doc.assets.insert(
        "picture".into(),
        touchbar_ui::Image::rgba8(1, 1, 1, 1, vec![255, 0, 0, 255]).unwrap(),
    );
    doc.asset_kinds
        .insert("picture".into(), touchbar_sets::AssetKind::Png);
    let png = draw(&scene, &docs);
    assert!(png.faults.is_empty());
    assert!(png.display_list.commands.iter().any(|c| matches!(
        c,
        touchbar_ui::DrawCommand::Image {
            coloring: touchbar_ui::ImageColoring::Original,
            ..
        }
    )));
    Arc::make_mut(docs.sets.values_mut().next().unwrap())
        .asset_kinds
        .insert("picture".into(), touchbar_sets::AssetKind::SymbolicSvg);
    let svg = draw(&scene, &docs);
    assert!(svg.display_list.commands.iter().any(|c| matches!(
        c,
        touchbar_ui::DrawCommand::Image {
            coloring: touchbar_ui::ImageColoring::Mask(_),
            ..
        }
    )));
}
