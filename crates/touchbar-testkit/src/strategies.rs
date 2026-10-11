//! Small stateful scenarios. Shrinking removes whole operations and shrinks their data.
use crate::*;
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use touchbar_core::*;
use touchbar_model::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GeneratedStep {
    Touch {
        surface: u8,
        contact: u8,
        phase: u8,
        widget: u8,
        dx: i16,
    },
    Advance(u16),
    Fn {
        surface: u8,
        pressed: bool,
    },
    App(u8),
    Mode {
        surface: u8,
        native: bool,
    },
    Remove(u8),
    Revoke(u8),
    Effect {
        success: bool,
    },
    Add(u8),
    Resize {
        surface: u8,
        width: u16,
        density: u8,
    },
    Catalog {
        revision: bool,
        all: bool,
    },
    Fault {
        surface: u8,
        hang: bool,
    },
    Stale,
    Subscription {
        surface: u8,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GeneratedScenario {
    pub profiles: u8,
    pub surfaces: Vec<SurfaceInfo>,
    pub logic_delay: u8,
    pub render_delay: u8,
    pub effect_delay: u16,
    pub all_layers: bool,
    pub probe_panel: bool,
    pub probe_pending: bool,
    pub probe_fault: bool,
    pub steps: Vec<GeneratedStep>,
}
pub fn scenario_strategy() -> impl Strategy<Value = GeneratedScenario> {
    let step = prop_oneof![
        7=>(0u8..2,1u8..4,0u8..4,0u8..4,-100i16..101).prop_map(|(surface,contact,phase,widget,dx)|GeneratedStep::Touch{surface,contact,phase,widget,dx}),
        3=>(0u16..1701).prop_map(GeneratedStep::Advance),
        2=>(0u8..2,any::<bool>()).prop_map(|(surface,pressed)|GeneratedStep::Fn{surface,pressed}),
        2=>(0u8..3).prop_map(GeneratedStep::App),
        1=>(0u8..2,any::<bool>()).prop_map(|(surface,native)|GeneratedStep::Mode{surface,native}),
        1=>(0u8..2).prop_map(GeneratedStep::Remove),
        1=>(0u8..2).prop_map(GeneratedStep::Revoke),
        1=>any::<bool>().prop_map(|success|GeneratedStep::Effect{success}),
        1=>(0u8..2).prop_map(GeneratedStep::Add),
        1=>(0u8..2,200u16..2201,2u8..11).prop_map(|(surface,width,density)|GeneratedStep::Resize{surface,width,density}),
        1=>(any::<bool>(),any::<bool>()).prop_map(|(revision,all)|GeneratedStep::Catalog{revision,all}),
        2=>(0u8..2,any::<bool>()).prop_map(|(surface,hang)|GeneratedStep::Fault{surface,hang}),
        1=>Just(GeneratedStep::Stale),
        1=>(0u8..2).prop_map(|surface|GeneratedStep::Subscription{surface}),
    ];
    (
        1u8..4,
        1usize..3,
        0u8..3,
        any::<bool>(),
        2u8..11,
        0u8..11,
        0u8..6,
        1u16..1001,
        (any::<bool>(), any::<bool>(), any::<bool>(), any::<bool>()),
        proptest::collection::vec(step, 1..49),
    )
        .prop_map(
            |(
                profiles,
                count,
                kind,
                inject,
                density,
                logic_delay,
                render_delay,
                effect_delay,
                (all_layers, probe_panel, probe_pending, probe_fault),
                steps,
            )| {
                let surfaces = (0..count)
                    .map(|index| {
                        let id = index as u64 + 1;
                        let mut info = headless(id);
                        info.kind =
                            match (kind + u8::try_from(index).expect("at most two surfaces")) % 3 {
                                0 => SurfaceKind::Touchbar,
                                1 => SurfaceKind::Window,
                                _ => SurfaceKind::Headless,
                            };
                        info.name = match info.kind {
                            SurfaceKind::Touchbar => "touchbar".parse().unwrap(),
                            SurfaceKind::Window => format!("window:test{id}").parse().unwrap(),
                            SurfaceKind::Headless => format!("headless:test{id}").parse().unwrap(),
                        };
                        info.width_px = if index == 0 { 2008 } else { 2170 };
                        info.height_px = if info.kind == SurfaceKind::Window {
                            120
                        } else {
                            60
                        };
                        info.px_per_mm = f32::from(density);
                        info.caps.modes = true;
                        info.caps.inject_keys = inject;
                        info
                    })
                    .collect();
                GeneratedScenario {
                    profiles,
                    surfaces,
                    logic_delay,
                    render_delay,
                    effect_delay,
                    all_layers,
                    probe_panel,
                    probe_pending,
                    probe_fault,
                    steps,
                }
            },
        )
}
impl GeneratedScenario {
    pub fn catalog(&self, revision: bool, all: bool) -> Catalog {
        let mut c = fixtures::controls_with_panel();
        c.sets[0].widgets[0]
            .state_bindings
            .insert(StateBinding::Text, "label".into());
        // Base controls also exercise the key fallback without activating Fn.
        c.sets[0].widgets.push(WidgetSpec::new(
            "escape".parse().unwrap(),
            WidgetKind::Key {
                key: SystemKey::Esc,
            },
        ));
        let mut alternate = c.sets[0].clone();
        alternate.reference = "controls/alternate".parse().unwrap();
        c.sets.push(alternate);
        for n in 1..self.profiles {
            let mut p = ProfileSpec::default_for(format!("controls/app{n}").parse().unwrap());
            p.for_target = Some(ProfileTarget::App(
                if n == 1 { "editor" } else { "terminal" }.into(),
            ));
            p.slots = vec![SlotSpec {
                id: "main".parse().unwrap(),
                candidates: vec!["controls/alternate".parse().unwrap()],
            }];
            c.profiles.push(p);
        }
        c.plugins[0].revision = if revision { "test-2" } else { "test-1" }.into();
        c.bindings[0].action.surfaces = if all {
            LayerSurfaces::All
        } else {
            LayerSurfaces::Source
        };
        assert!(
            c.validate()
                .iter()
                .all(|i| i.code == CatalogIssueCode::ProfileConflict)
        );
        c
    }
    pub fn run(&self) -> Harness {
        let mut h = Harness::new(CoreConfig::default());
        h.enable_invariant_checks();
        h.effects.delay_ms = u64::from(self.effect_delay);
        h.renderer.delay_ms = u64::from(self.render_delay);
        let pending: Rc<RefCell<BTreeMap<InstanceId, bool>>> = Rc::default();
        for set in ["controls/main", "controls/alternate"] {
            let pending = pending.clone();
            let delay = self.logic_delay;
            let mut ticks: BTreeMap<InstanceId, u64> = BTreeMap::new();
            h.logic.register(set, None, move |id, call| {
                if let Some(hang) = pending.borrow_mut().remove(&id) {
                    return if hang {
                        LogicReaction::Hang
                    } else {
                        LogicReaction::Fault("generated trap".into())
                    };
                }
                let mut update = LogicUpdate::default();
                if matches!(
                    call,
                    LogicCall::Init { .. } | LogicCall::Handle(LogicEvent::Shown)
                ) {
                    update.subscriptions = Some(vec![SubscriptionSpec {
                        tag: "tick".into(),
                        kind: SubscriptionKind::TimerInterval { ms: 512 },
                        background: false,
                    }]);
                }
                if matches!(call, LogicCall::Handle(LogicEvent::SubscriptionData { .. })) {
                    let tick = ticks.entry(id).or_default();
                    *tick += 1;
                    update.state = vec![(
                        "label".into(),
                        StateValue::Scalar(Scalar::Text(tick.to_string())),
                    )];
                    // Desired subscriptions are repeated, not implicitly preserved by the fake.
                    update.subscriptions = Some(vec![SubscriptionSpec {
                        tag: "tick".into(),
                        kind: SubscriptionKind::TimerInterval { ms: 512 },
                        background: false,
                    }]);
                }
                if let LogicCall::Handle(LogicEvent::Action(a)) = call {
                    let effect = if let Some(Scalar::Number(level)) = a.value {
                        Some(Effect::AudioSetVolume {
                            target: AudioTarget::DefaultSink,
                            level,
                        })
                    } else if a.kind == ActionKind::Activated {
                        Some(Effect::AudioSetMute {
                            target: AudioTarget::DefaultSource,
                            value: MuteValue::Toggle,
                        })
                    } else {
                        None
                    };
                    if let Some(effect) = effect {
                        update.requests.push(EffectRequest {
                            tag: "write".into(),
                            effect,
                            lifetime: None,
                            coalesce: Some("write".into()),
                            timeout_ms: None,
                        });
                    }
                }
                if delay == 0 {
                    LogicReaction::Update(update)
                } else {
                    LogicReaction::Delayed {
                        ms: u64::from(delay),
                        update,
                    }
                }
            });
        }
        h.send(Input::DesktopCapabilities(DesktopCapabilities {
            key: true,
        }));
        h.send(Input::Catalog(self.catalog(false, self.all_layers)));
        for info in &self.surfaces {
            h.send(Input::SurfaceAdded(info.clone()));
            h.send(Input::Mode {
                surface: info.id,
                mode: Mode::Display,
            });
        }
        h.advance_ms(50);
        let mut contacts: BTreeMap<(SurfaceId, u8), Point> = BTreeMap::new();
        if self.probe_panel {
            let id = self.surfaces[0].id;
            let point = Point { x: 120.0, y: 30.0 };
            h.touch(id, 1, TouchPhase::Down, point);
            contacts.insert((id, 1), point);
            h.advance_ms(110);
        }
        if self.probe_pending {
            let id = self.surfaces[0].id;
            contacts.entry((id, 1)).or_insert_with(|| {
                let point = Point { x: 120.0, y: 30.0 };
                h.touch(id, 1, TouchPhase::Down, point);
                point
            });
            h.send(Input::Fact {
                name: "app.id".into(),
                value: Some(FactValue::Text("editor".into())),
            });
        }
        if self.probe_fault {
            fault(&mut h, &pending, self.surfaces[0].id, false);
            h.advance_ms(60);
        }
        for (number, step) in self.steps.iter().enumerate() {
            if h.invariant_error.is_some() {
                break;
            }
            let choose = |n: u8| self.surfaces[usize::from(n) % self.surfaces.len()].id;
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
                    let info = self.surfaces[usize::from(n) % self.surfaces.len()].clone();
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
                    h.send(Input::Catalog(self.catalog(revision, all)));
                }
                GeneratedStep::Fault { surface, hang } => {
                    fault(&mut h, &pending, choose(surface), hang)
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
                        h.subscription_after(id, SubscriptionData::Tick, 1);
                        h.advance_ms(1);
                    }
                }
            }
            // Distinct locale values can force new context calls after a hang/fault script.
            if number % 7 == 0 {
                h.send(Input::Locale(format!("test-{number}")));
            }
        }
        for info in &self.surfaces {
            h.send(Input::SurfaceRemoved(info.id));
        }
        h.advance_ms(u64::from(self.effect_delay) + 1);
        h
    }
}
fn fault(
    h: &mut Harness,
    pending: &Rc<RefCell<BTreeMap<InstanceId, bool>>>,
    surface: SurfaceId,
    hang: bool,
) {
    let snap = h.snapshot();
    if let Some(i) = snap.instances.iter().find(|i| {
        i.surface == surface
            && matches!(i.status, InstanceStatus::Visible | InstanceStatus::Loading)
            && i.set.id() != "fkeys"
    }) {
        if !hang && let Some(call) = i.in_flight {
            h.send(Input::LogicFault {
                instance: i.id,
                call,
                reason: "generated trap".into(),
            });
        } else {
            pending.borrow_mut().insert(i.id, hang);
            h.send(Input::Locale(format!(
                "fault-{}-{}",
                i.id.number,
                h.now().0
            )));
        }
    }
}
