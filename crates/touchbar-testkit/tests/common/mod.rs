use std::collections::BTreeMap;
use touchbar_core::*;
use touchbar_model::*;
use touchbar_testkit::{strategies::*, *};
pub fn controls_host() -> (LoadedPlugin, Option<WasmLogic>) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/controls");
    let plugin = plugin_dir(root).unwrap();
    let mut host = WasmLogic::new();
    host.register_plugin(&plugin).unwrap();
    let bytes = plugin.component.as_ref().unwrap();
    host.register_bytes("controls", "test-2", bytes).unwrap();
    host.register("controls", "fault-trap", touchbar_wasm_fixtures::TRAP)
        .unwrap();
    host.register("controls", "fault-hang", touchbar_wasm_fixtures::HANG)
        .unwrap();
    (plugin, Some(host))
}
pub fn controls_catalog(plugin: &LoadedPlugin, profiles: u8, revision: bool, all: bool) -> Catalog {
    let mut c = plugin.catalog.clone();
    if revision {
        c.plugins[0].revision = "test-2".into();
    }
    let system =
        plugin_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/system"))
            .unwrap();
    c.plugins.extend(system.catalog.plugins);
    c.sets.extend(system.catalog.sets);
    for n in 0..profiles {
        let mut p = ProfileSpec::default_for(format!("controls/profile{n}").parse().unwrap());
        p.default = n == 0;
        if n > 0 {
            p.for_target = Some(ProfileTarget::App(
                if n == 1 { "editor" } else { "terminal" }.into(),
            ));
        }
        p.slots = ["volume", "microphone", "brightness"]
            .into_iter()
            .map(|name| SlotSpec {
                id: name.parse().unwrap(),
                candidates: vec![format!("controls/{name}").parse().unwrap()],
            })
            .collect();
        // Changing slot placement forces a genuine pending profile transition.
        if n > 0 {
            p.slots[0].id = format!("volume{n}").parse().unwrap();
        }
        c.profiles.push(p);
    }
    c.bindings.push(Binding {
        id: "fn".into(),
        when: None,
        trigger: Trigger::Key(KeyTrigger {
            keys: vec![WatchKey::Fn],
            gesture: KeyGesture::WhileHeld,
        }),
        action: LayerAction {
            operation: LayerOperation::Layer("system/fkeys".parse().unwrap()),
            surfaces: if all {
                LayerSurfaces::All
            } else {
                LayerSurfaces::Source
            },
        },
    });
    assert!(c.validate().is_empty(), "{:?}", c.validate());
    c
}
pub fn seed_subscriptions(h: &mut Harness<WasmLogic>) {
    for subscription in h.snapshot().subscriptions {
        if let Some(id) = subscription.active {
            let data = match subscription.spec.kind {
                SubscriptionKind::AudioWatch { .. } => SubscriptionData::Audio(AudioState {
                    available: true,
                    volume: 0.4,
                    muted: false,
                    name: "test".into(),
                }),
                SubscriptionKind::DesktopFact { .. } => {
                    SubscriptionData::Fact(Some(FactValue::Number(0.5)))
                }
                _ => continue,
            };
            h.send(Input::SubscriptionData {
                subscription: id,
                data,
            });
        }
    }
}
pub fn run_controls(
    case: &GeneratedScenario,
    host: WasmLogic,
    plugin: &LoadedPlugin,
) -> Harness<WasmLogic> {
    let mut h = Harness::with_logic(CoreConfig::default(), host);
    h.enable_invariant_checks();
    h.effects.delay_ms = u64::from(case.effect_delay);
    h.renderer.delay_ms = u64::from(case.render_delay);
    h.send(Input::DesktopCapabilities(DesktopCapabilities {
        key: true,
    }));
    h.send(Input::Catalog(controls_catalog(
        plugin,
        case.profiles,
        false,
        case.all_layers,
    )));
    for info in &case.surfaces {
        h.send(Input::SurfaceAdded(info.clone()));
        h.send(Input::Mode {
            surface: info.id,
            mode: Mode::Display,
        });
    }
    h.advance_ms(50);
    assert!(
        h.snapshot()
            .instances
            .iter()
            .filter(|i| i.set.plugin.as_str() == "controls")
            .all(|i| i.status == InstanceStatus::Visible),
        "controls failed to initialize"
    );
    seed_subscriptions(&mut h);
    h.advance_ms(50);
    let mut contacts: BTreeMap<(SurfaceId, u8), Point> = BTreeMap::new();
    if case.probe_panel {
        let id = case.surfaces[0].id;
        let point = h.hit(id, "volume-panel");
        h.touch(id, 1, TouchPhase::Down, point);
        contacts.insert((id, 1), point);
        h.advance_ms(421);
        h.advance_ms(50);
    }
    if case.probe_pending {
        let id = case.surfaces[0].id;
        contacts.entry((id, 1)).or_insert_with(|| {
            let point = h.hit(id, "volume-panel");
            h.touch(id, 1, TouchPhase::Down, point);
            point
        });
        h.send(Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("editor".into())),
        });
    }
    if case.probe_fault {
        generated_fault(&mut h, plugin, case, false);
        h.advance_ms(60);
    }
    for (number, step) in case.steps.iter().enumerate() {
        if h.invariant_error.is_some() {
            break;
        }
        let choose = |n: u8| case.surfaces[usize::from(n) % case.surfaces.len()].id;
        match *step {
            GeneratedStep::Touch {
                surface,
                contact,
                phase,
                widget,
                dx,
            } => {
                let id = choose(surface);
                if let Some(info) = h
                    .snapshot()
                    .surfaces
                    .iter()
                    .find(|s| s.info.id == id)
                    .map(|s| s.info.clone())
                {
                    let point = Point {
                        x: f32::from(widget) * 80.0 + 40.0 + f32::from(dx),
                        y: info.height_px as f32 / 2.0,
                    };
                    let active = contacts.contains_key(&(id, contact));
                    let phase = if !active {
                        TouchPhase::Down
                    } else {
                        match phase {
                            0 | 1 => TouchPhase::Move,
                            2 => TouchPhase::Up,
                            _ => TouchPhase::Cancel,
                        }
                    };
                    h.touch(id, u32::from(contact), phase, point);
                    if matches!(phase, TouchPhase::Down | TouchPhase::Move) {
                        contacts.insert((id, contact), point);
                    } else {
                        contacts.remove(&(id, contact));
                    }
                }
            }
            GeneratedStep::Advance(ms) => h.advance_ms(u64::from(ms)),
            GeneratedStep::Fn { surface, pressed } => {
                h.send(Input::Key {
                    source: KeySource::Surface(choose(surface)),
                    code: WatchKey::Fn,
                    phase: if pressed {
                        KeyPhase::Pressed
                    } else {
                        KeyPhase::Released
                    },
                });
            }
            GeneratedStep::App(n) => {
                h.send(Input::Fact {
                    name: "app.id".into(),
                    value: Some(FactValue::Text(
                        match n {
                            0 => "none",
                            1 => "editor",
                            _ => "terminal",
                        }
                        .into(),
                    )),
                });
            }
            GeneratedStep::Mode { surface, native } => {
                h.send(Input::Mode {
                    surface: choose(surface),
                    mode: if native { Mode::Native } else { Mode::Display },
                });
            }
            GeneratedStep::Remove(n) => {
                let id = choose(n);
                h.send(Input::SurfaceRemoved(id));
                contacts.retain(|(s, _), _| *s != id);
            }
            GeneratedStep::Revoke(n) => {
                let id = choose(n);
                h.send(Input::Revoked { surface: id });
                contacts.retain(|(s, _), _| *s != id);
            }
            GeneratedStep::Effect { success } => {
                if let Some(request) = h
                    .snapshot()
                    .requests
                    .iter()
                    .find(|r| r.running)
                    .map(|r| r.id)
                {
                    h.send(Input::EffectDone {
                        request,
                        result: if success {
                            Ok(EffectResult::None)
                        } else {
                            Err(EffectError::Failed("generated executor error".into()))
                        },
                    });
                }
            }
            GeneratedStep::Add(n) => {
                let info = case.surfaces[usize::from(n) % case.surfaces.len()].clone();
                let id = info.id;
                h.send(Input::SurfaceAdded(info));
                h.send(Input::Mode {
                    surface: id,
                    mode: Mode::Display,
                });
            }
            GeneratedStep::Resize {
                surface,
                width,
                density,
            } => {
                if let Some(mut info) = h
                    .snapshot()
                    .surfaces
                    .iter()
                    .find(|s| s.info.id == choose(surface))
                    .map(|s| s.info.clone())
                {
                    info.width_px = u32::from(width);
                    info.px_per_mm = f32::from(density);
                    h.send(Input::SurfaceChanged(info));
                }
            }
            GeneratedStep::Catalog { revision, all } => {
                h.send(Input::Catalog(controls_catalog(
                    plugin,
                    case.profiles,
                    revision,
                    all,
                )));
            }
            GeneratedStep::Fault { surface, hang } => {
                generated_fault(&mut h, plugin, case, hang);
                let _ = surface;
            }
            GeneratedStep::Stale => {
                let dead = InstanceId {
                    number: u64::MAX,
                    generation: Generation(1),
                };
                h.send(Input::LogicUpdate {
                    instance: dead,
                    call: CallId(u64::MAX),
                    update: LogicUpdate::default(),
                });
                h.send(Input::EffectDone {
                    request: RequestId {
                        instance: dead,
                        number: u64::MAX,
                    },
                    result: Ok(EffectResult::None),
                });
                h.send(Input::SubscriptionData {
                    subscription: SubscriptionId {
                        instance: dead,
                        number: u64::MAX,
                    },
                    data: SubscriptionData::Tick,
                });
            }
            GeneratedStep::Subscription { surface } => {
                let snap = h.snapshot();
                if let Some(id) = snap
                    .subscriptions
                    .iter()
                    .find(|sub| {
                        sub.active.is_some()
                            && snap
                                .instances
                                .iter()
                                .any(|i| i.id == sub.owner && i.surface == choose(surface))
                    })
                    .and_then(|s| s.active)
                {
                    let spec = snap
                        .subscriptions
                        .iter()
                        .find(|s| s.active == Some(id))
                        .unwrap();
                    let data = match spec.spec.kind {
                        SubscriptionKind::AudioWatch { .. } => {
                            SubscriptionData::Audio(AudioState {
                                available: true,
                                volume: 0.8,
                                muted: true,
                                name: "external".into(),
                            })
                        }
                        _ => SubscriptionData::Fact(Some(FactValue::Number(0.7))),
                    };
                    h.subscription_after(id, data, 1);
                    h.advance_ms(1);
                }
            }
        }
        // Distinct locale values can force new context calls after a hang/fault script.
        if number % 7 == 0 {
            h.send(Input::Locale(format!("test-{number}")));
        }
    }
    for info in &case.surfaces {
        h.send(Input::SurfaceRemoved(info.id));
    }
    h.advance_ms(u64::from(case.effect_delay) + 1);
    h
}
fn generated_fault(
    h: &mut Harness<WasmLogic>,
    plugin: &LoadedPlugin,
    case: &GeneratedScenario,
    hang: bool,
) {
    let mut c = controls_catalog(plugin, case.profiles, false, case.all_layers);
    c.plugins[0].revision = if hang { "fault-hang" } else { "fault-trap" }.into();
    h.send(Input::Catalog(c));
}
