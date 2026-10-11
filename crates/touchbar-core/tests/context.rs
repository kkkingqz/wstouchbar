use touchbar_core::*;
use touchbar_model::*;

fn catalog() -> Catalog {
    let mut set = SetSpec::default_for("p/main".parse().unwrap());
    set.logic = Some("p".parse().unwrap());
    let mut profile = ProfileSpec::default_for("p/default".parse().unwrap());
    profile.default = true;
    profile.slots.push(SlotSpec {
        id: "main".parse().unwrap(),
        candidates: vec![set.reference.clone()],
    });
    Catalog {
        plugins: vec![PluginSpec {
            id: "p".parse().unwrap(),
            revision: "1".into(),
            enabled: true,
            has_logic: true,
            requires: vec![],
        }],
        sets: vec![set],
        profiles: vec![profile],
        ..Catalog::default()
    }
}
fn info(id: u64) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: format!("headless:{id}").parse().unwrap(),
        kind: SurfaceKind::Headless,
        width_px: 500,
        height_px: 50,
        px_per_mm: 5.0,
        caps: SurfaceCaps::default(),
    }
}
fn setup() -> (Core, InstanceId) {
    let mut c = Core::new(CoreConfig::default());
    send(&mut c, Input::Catalog(catalog()));
    send(&mut c, Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    (c, id)
}
fn send(c: &mut Core, input: Input) -> Vec<Output> {
    c.handle(Timestamp(0), input)
}
fn describe(c: &mut Core, id: InstanceId) -> Vec<Output> {
    send(
        c,
        Input::LogicDescribed {
            instance: id,
            description: LogicDescription::default(),
            persisted: vec![],
        },
    )
}
fn reply(c: &mut Core, id: InstanceId) -> Vec<Output> {
    let call = c
        .snapshot()
        .instances
        .iter()
        .find(|i| i.id == id)
        .unwrap()
        .in_flight
        .unwrap();
    send(
        c,
        Input::LogicUpdate {
            instance: id,
            call,
            update: LogicUpdate::default(),
        },
    )
}
fn ready(c: &mut Core, id: InstanceId) {
    describe(c, id);
    reply(c, id);
    reply(c, id);
}
fn layout(c: &Core, id: InstanceId, width: f32, representation: &str) -> Input {
    let s = c
        .snapshot()
        .surfaces
        .into_iter()
        .find(|s| s.scene.items.iter().any(|i| i.instance == id))
        .unwrap();
    Input::Layout {
        surface: s.info.id,
        generation: s.generation,
        hit_map: vec![],
        representations: if representation.is_empty() {
            vec![]
        } else {
            vec![(id, representation.into())]
        },
        scrubbers: vec![],
        slots: vec![SlotBounds {
            instance: id,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: 50.0,
            },
        }],
        presentation: None,
    }
}
fn contexts(out: &[Output]) -> Vec<(InstanceId, LogicContext)> {
    out.iter()
        .filter_map(|o| {
            if let Output::CallLogic {
                instance,
                invocation: LogicCall::Handle(LogicEvent::ContextChanged(ctx)),
                ..
            } = o
            {
                Some((*instance, ctx.clone()))
            } else {
                None
            }
        })
        .collect()
}
fn expected(width: f32, representation: &str) -> LogicContext {
    LogicContext {
        surface_kind: "headless".into(),
        slot_width: width,
        representation: representation.into(),
        color_scheme: "dark".into(),
        accent: None,
        locale: "C".into(),
    }
}
#[test]
fn first_context_requires_both_init_success_and_accepted_slot_geometry() {
    let (mut c, id) = setup();
    let input = layout(&c, id, 100.0, "");
    assert!(contexts(&send(&mut c, input)).is_empty());
    assert!(contexts(&describe(&mut c, id)).is_empty());
    let mut out = reply(&mut c, id);
    if contexts(&out).is_empty() {
        out = reply(&mut c, id);
    }
    assert_eq!(contexts(&out), vec![(id, expected(200.0, ""))]);
}
#[test]
fn init_without_layout_does_not_invent_width_or_representation() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    assert!(c.snapshot().instances[0].in_flight.is_none());
    let input = layout(&c, id, 120.0, "compact");
    assert_eq!(
        contexts(&send(&mut c, input)),
        vec![(id, expected(240.0, "compact"))]
    );
}
#[test]
fn equal_layout_and_theme_do_not_repeat_context_and_changes_do() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    let input = layout(&c, id, 100.0, "");
    send(&mut c, input);
    reply(&mut c, id);
    let input = layout(&c, id, 100.0, "");
    assert!(contexts(&send(&mut c, input)).is_empty());
    assert!(
        contexts(&send(
            &mut c,
            Input::Fact {
                name: "appearance.scheme".into(),
                value: Some(FactValue::Text("dark".into()))
            }
        ))
        .is_empty()
    );
    let mut ctx = expected(200.0, "");
    ctx.color_scheme = "light".into();
    assert_eq!(
        contexts(&send(
            &mut c,
            Input::Fact {
                name: "appearance.scheme".into(),
                value: Some(FactValue::Text("light".into()))
            }
        )),
        vec![(id, ctx.clone())]
    );
    reply(&mut c, id);
    ctx.accent = Some("#abcdef".into());
    assert_eq!(
        contexts(&send(
            &mut c,
            Input::Fact {
                name: "appearance.accent".into(),
                value: Some(FactValue::Text("#abcdef".into()))
            }
        )),
        vec![(id, ctx)]
    );
}
#[test]
fn stale_layout_and_missing_slot_do_not_create_context() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    let mut input = layout(&c, id, 100.0, "");
    if let Input::Layout { generation, .. } = &mut input {
        *generation = Generation(generation.0 + 99);
    }
    assert!(contexts(&send(&mut c, input)).is_empty());
    let mut input = layout(&c, id, 100.0, "");
    if let Input::Layout { slots, .. } = &mut input {
        slots.clear();
    }
    assert!(contexts(&send(&mut c, input)).is_empty());
    let input = layout(&c, id, 100.0, "");
    assert_eq!(contexts(&send(&mut c, input)).len(), 1);
}
#[test]
fn geometry_updates_only_target_instance_on_its_surface_and_use_set_units() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    send(&mut c, Input::SurfaceAdded(info(2)));
    let other = c.snapshot().instances[1].id;
    ready(&mut c, other);
    let input = layout(&c, id, 100.0, "wide");
    assert_eq!(
        contexts(&send(&mut c, input)),
        vec![(id, expected(200.0, "wide"))]
    );
    let input = layout(&c, other, 50.0, "small");
    assert_eq!(
        contexts(&send(&mut c, input)),
        vec![(other, expected(100.0, "small"))]
    );
}
#[test]
fn locale_is_explicit_checkpointed_and_invalid_values_are_ignored() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    let input = layout(&c, id, 100.0, "");
    send(&mut c, input);
    reply(&mut c, id);
    let mut ctx = expected(200.0, "");
    ctx.locale = "uk_UA.UTF-8".into();
    assert_eq!(
        contexts(&send(&mut c, Input::Locale(ctx.locale.clone()))),
        vec![(id, ctx)]
    );
    reply(&mut c, id);
    assert!(contexts(&send(&mut c, Input::Locale(String::new()))).is_empty());
    let json = serde_json::to_vec(&c.checkpoint()).unwrap();
    let mut restored = Core::restore(serde_json::from_slice(&json).unwrap()).unwrap();
    let input = layout(&c, id, 100.0, "");
    assert!(contexts(&send(&mut restored, input.clone())).is_empty());
    send(&mut c, input);
    let input = Input::Locale("ru_RU.UTF-8".into());
    assert_eq!(send(&mut c, input.clone()), send(&mut restored, input));
}
#[test]
fn replacement_generation_receives_context_even_when_values_match() {
    let (mut c, id) = setup();
    ready(&mut c, id);
    let input = layout(&c, id, 100.0, "");
    send(&mut c, input);
    reply(&mut c, id);
    let mut cat = catalog();
    cat.plugins[0].revision = "2".into();
    send(&mut c, Input::Catalog(cat));
    let new = c.snapshot().instances[0].id;
    ready(&mut c, new);
    let input = layout(&c, new, 100.0, "");
    assert_eq!(
        contexts(&send(&mut c, input)),
        vec![(new, expected(200.0, ""))]
    );
}
#[test]
fn context_respects_custom_units_and_rejects_unrepresentable_width() {
    let mut cat = catalog();
    cat.sets[0].units = Some(UnitSize {
        value: 2.0,
        axis: Axis::Width,
    });
    let mut c = Core::new(CoreConfig::default());
    send(&mut c, Input::Catalog(cat));
    send(&mut c, Input::SurfaceAdded(info(1)));
    let id = c.snapshot().instances[0].id;
    ready(&mut c, id);
    let input = layout(&c, id, 100.0, "");
    assert_eq!(
        contexts(&send(&mut c, input)),
        vec![(id, expected(10.0, ""))]
    );
    let mut cat = catalog();
    cat.plugins[0].revision = "2".into();
    cat.sets[0].units = Some(UnitSize {
        value: f32::MIN_POSITIVE / 100.0,
        axis: Axis::Height,
    });
    send(&mut c, Input::Catalog(cat));
    let id = c.snapshot().instances[0].id;
    ready(&mut c, id);
    let input = layout(&c, id, 500.0, "");
    let out = send(&mut c, input);
    assert!(contexts(&out).is_empty());
    assert!(
        out.iter()
            .any(|o| matches!(o,Output::Log {event,..} if event.contains("context width")))
    );
    serde_json::to_vec(&c.checkpoint()).unwrap();
}
