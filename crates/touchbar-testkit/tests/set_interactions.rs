use touchbar_core::*;
use touchbar_model::*;
use touchbar_sets::{PluginManifest, SetFile};
use touchbar_testkit::*;

#[test]
fn toml_scrubber_selection_reaches_logic_with_record_key() {
    let description = LogicDescription {
        actions: vec![(
            "choose".into(),
            ActionDescription {
                kinds: vec![ActionKind::Selected],
                value: None,
            },
        )],
        state: vec![StateField {
            name: "chosen".into(),
            kind: FieldKind::Scalar(ScalarKind::Text),
            persist: false,
        }],
    };
    let set = SetFile::from_toml_with_description("picker.toml", "version=1\nid='main'\nlabel='Picker'\n[layout]\nkind='scrubber'\nid='picker'\non_selected='choose'\n", &description).unwrap();
    let manifest = PluginManifest::from_toml("plugin.toml", "id='owner'\nversion='1.0.0'\napi='1'\nname='Picker'\ndescription='Selection'\nlogic='logic.wasm'").unwrap();
    let spec = set.to_spec(&manifest).unwrap();
    let c = catalog()
        .plugin("owner")
        .set("main", |s| s.spec = spec)
        .profile("default", |p| {
            p.default().slot("main", &["owner/main"]);
        })
        .build();
    let mut h = Harness::new(CoreConfig::default());
    h.enable_invariant_checks();
    h.logic
        .register("owner/main", Some(description), |_, call| {
            LogicReaction::Update(LogicUpdate {
                state: match call {
                    LogicCall::Handle(LogicEvent::Action(e)) if e.action == "choose" => vec![(
                        "chosen".into(),
                        StateValue::Scalar(Scalar::Text(e.record_key.clone().unwrap())),
                    )],
                    _ => vec![],
                },
                ..Default::default()
            })
        });
    h.send(Input::Catalog(c));
    h.send(Input::SurfaceAdded(headless(1)));
    let snapshot = h.snapshot();
    let instance = snapshot.instances[0].id;
    assert_eq!(snapshot.instances[0].status, InstanceStatus::Visible);
    // Item geometry comes from the renderer in stage 3; supply it explicitly here.
    h.renderer.delay_ms = 60_000;
    let rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 80.0,
        height: 50.0,
    };
    h.send(Input::Layout {
        surface: SurfaceId(1),
        generation: snapshot.surfaces[0].generation,
        slots: vec![SlotBounds { instance, rect }],
        presentation: None,
        representations: vec![],
        hit_map: vec![HitTarget {
            instance,
            widget: "picker".parse().unwrap(),
            plane: ScenePlane::Profile,
            rect,
            record_key: None,
        }],
        scrubbers: vec![ScrubberLayout {
            instance,
            widget: "picker".parse().unwrap(),
            viewport: rect,
            content_width: 80.0,
            items: vec![ScrubberItemGeometry {
                record_key: "alpha/beta".into(),
                rect,
            }],
        }],
    });
    h.tap(SurfaceId(1), "picker");
    let actions: Vec<_> = h
        .logic
        .calls
        .iter()
        .filter_map(|(_, _, call)| {
            if let LogicCall::Handle(LogicEvent::Action(e)) = call {
                Some(e)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        (
            actions[0].action.as_str(),
            actions[0].kind,
            actions[0].record_key.as_deref()
        ),
        ("choose", ActionKind::Selected, Some("alpha/beta"))
    );
    assert_eq!(
        h.snapshot().instances[0].state,
        vec![(
            "chosen".into(),
            StateValue::Scalar(Scalar::Text("alpha/beta".into()))
        )]
    );
    h.send(Input::SurfaceRemoved(SurfaceId(1)));
    h.finish_invariant_checks().unwrap();
}
