use touchbar_core::*;
use touchbar_model::*;
fn info(id: u64, modes: bool) -> SurfaceInfo {
    SurfaceInfo {
        id: SurfaceId(id),
        name: if id == 1 {
            "touchbar"
        } else {
            "headless:second"
        }
        .parse()
        .unwrap(),
        kind: if id == 1 {
            SurfaceKind::Touchbar
        } else {
            SurfaceKind::Headless
        },
        width_px: 2008,
        height_px: 60,
        px_per_mm: 6.0,
        caps: SurfaceCaps {
            modes,
            ..SurfaceCaps::default()
        },
    }
}
fn render_generation(outputs: &[Output]) -> Generation {
    outputs
        .iter()
        .find_map(|o| match o {
            Output::Render { generation, .. } => Some(*generation),
            _ => None,
        })
        .unwrap()
}
#[test]
fn mode_capable_surface_waits_for_display() {
    let mut c = Core::new(CoreConfig::default());
    let output = c.handle(Timestamp(0), Input::SurfaceAdded(info(1, true)));
    assert!(!output.iter().any(|o| matches!(o, Output::Render { .. })));
    assert_eq!(c.snapshot().surfaces[0].mode, Mode::Native);
    let output = c.handle(
        Timestamp(1),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    );
    assert!(output.iter().any(|o| matches!(
        o,
        Output::Render {
            surface: SurfaceId(1),
            ..
        }
    )));
}
#[test]
fn ordinary_surface_rejects_native_mode() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let output = c.handle(
        Timestamp(1),
        Input::Mode {
            surface: SurfaceId(2),
            mode: Mode::Native,
        },
    );
    assert!(output.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
    assert_eq!(c.snapshot().surfaces[0].mode, Mode::Display);
}
#[test]
fn changed_geometry_is_coalesced_until_frame_done() {
    let mut c = Core::new(CoreConfig::default());
    let first = c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let generation = render_generation(&first);
    let mut updated = info(2, false);
    updated.width_px = 2170;
    assert!(
        !c.handle(Timestamp(1), Input::SurfaceChanged(updated))
            .iter()
            .any(|o| matches!(o, Output::Render { .. }))
    );
    let output = c.handle(
        Timestamp(2),
        Input::FrameDone {
            surface: SurfaceId(2),
            generation,
        },
    );
    assert!(render_generation(&output) > generation);
    assert_eq!(c.snapshot().surfaces[0].info.width_px, 2170);
}
#[test]
fn native_ignores_late_layout_and_frame_done() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1, true)));
    let generation = render_generation(&c.handle(
        Timestamp(1),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
    ));
    c.handle(
        Timestamp(2),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Native,
        },
    );
    let layout = Input::Layout {
        surface: SurfaceId(1),
        generation,
        hit_map: vec![],
        representations: vec![],
    };
    assert!(c.handle(Timestamp(3), layout).is_empty());
    assert!(
        c.handle(
            Timestamp(4),
            Input::FrameDone {
                surface: SurfaceId(1),
                generation
            }
        )
        .is_empty()
    );
    assert_eq!(c.snapshot().surfaces[0].layout_generation, None);
}
#[test]
fn reconnect_reuses_id_but_not_render_generation() {
    let mut c = Core::new(CoreConfig::default());
    let first = render_generation(&c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false))));
    c.handle(Timestamp(1), Input::SurfaceRemoved(SurfaceId(2)));
    let second = render_generation(&c.handle(Timestamp(2), Input::SurfaceAdded(info(2, false))));
    assert!(second > first);
    c.handle(
        Timestamp(3),
        Input::Layout {
            surface: SurfaceId(2),
            generation: first,
            hit_map: vec![],
            representations: vec![],
        },
    );
    assert_eq!(c.snapshot().surfaces[0].layout_generation, None);
}
#[test]
fn revocation_removes_only_its_surface() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1, true)));
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let output = c.handle(
        Timestamp(1),
        Input::Revoked {
            surface: SurfaceId(1),
        },
    );
    assert!(!output.iter().any(|o| matches!(
        o,
        Output::Render {
            surface: SurfaceId(2),
            ..
        }
    )));
    assert_eq!(c.snapshot().surfaces.len(), 1);
    assert_eq!(c.snapshot().surfaces[0].info.id, SurfaceId(2));
}
#[test]
fn invalid_surface_does_not_replace_previous_info() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let mut bad = info(2, false);
    bad.px_per_mm = 0.0;
    assert!(
        c.handle(Timestamp(1), Input::SurfaceChanged(bad))
            .iter()
            .any(|o| matches!(
                o,
                Output::Log {
                    level: LogLevel::Warn,
                    ..
                }
            ))
    );
    assert_eq!(c.snapshot().surfaces[0].info.px_per_mm, 6.0);
}
#[test]
fn facts_are_validated_and_can_be_removed() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(
        Timestamp(0),
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("firefox".into())),
        },
    );
    assert_eq!(
        c.snapshot().facts[&FactName::from("app.id")],
        FactValue::Text("firefox".into())
    );
    c.handle(
        Timestamp(1),
        Input::Fact {
            name: "bad".into(),
            value: Some(FactValue::Number(f64::NAN)),
        },
    );
    assert!(!c.snapshot().facts.contains_key(&FactName::from("bad")));
    c.handle(
        Timestamp(2),
        Input::Fact {
            name: "app.id".into(),
            value: None,
        },
    );
    assert!(c.snapshot().facts.is_empty());
}
#[test]
fn catalog_is_retained_if_replacement_is_invalid() {
    let mut c = Core::new(CoreConfig::default());
    let good = Catalog {
        plugins: vec![PluginSpec {
            id: "test".parse().unwrap(),
            enabled: true,
            has_logic: false,
            requires: vec![],
        }],
        ..Catalog::default()
    };
    c.handle(Timestamp(0), Input::Catalog(good.clone()));
    let mut bad = good;
    bad.plugins.push(bad.plugins[0].clone());
    assert!(
        c.handle(Timestamp(1), Input::Catalog(bad))
            .iter()
            .any(|o| matches!(
                o,
                Output::Log {
                    level: LogLevel::Warn,
                    ..
                }
            ))
    );
    assert_eq!(c.snapshot().plugins.len(), 1);
}
#[test]
fn unknown_timer_is_harmless() {
    let mut c = Core::new(CoreConfig::default());
    assert!(
        c.handle(Timestamp(0), Input::Timer { id: TimerId(123) })
            .is_empty()
    );
}
#[test]
fn same_input_sequence_has_same_outputs_and_snapshot() {
    let inputs = vec![
        Input::SurfaceAdded(info(1, true)),
        Input::SurfaceAdded(info(2, false)),
        Input::Mode {
            surface: SurfaceId(1),
            mode: Mode::Display,
        },
        Input::Fact {
            name: "app.id".into(),
            value: Some(FactValue::Text("firefox".into())),
        },
        Input::SurfaceRemoved(SurfaceId(2)),
    ];
    let mut a = Core::new(CoreConfig::default());
    let mut b = Core::new(CoreConfig::default());
    for (n, input) in inputs.into_iter().enumerate() {
        let now = Timestamp(u64::try_from(n).unwrap());
        assert_eq!(a.handle(now, input.clone()), b.handle(now, input));
    }
    assert_eq!(a.snapshot(), b.snapshot());
    let encoded = serde_json::to_string(&a.snapshot()).unwrap();
    assert_eq!(
        serde_json::from_str::<CoreSnapshot>(&encoded).unwrap(),
        a.snapshot()
    );
}
#[test]
fn duplicate_surface_name_does_not_add_another_surface() {
    let mut c = Core::new(CoreConfig::default());
    c.handle(Timestamp(0), Input::SurfaceAdded(info(2, false)));
    let mut duplicate = info(2, false);
    duplicate.id = SurfaceId(3);
    assert!(
        c.handle(Timestamp(1), Input::SurfaceAdded(duplicate))
            .iter()
            .any(|o| matches!(
                o,
                Output::Log {
                    level: LogLevel::Warn,
                    ..
                }
            ))
    );
    assert_eq!(c.snapshot().surfaces.len(), 1);
}
#[test]
fn reserved_surface_facts_cannot_override_derived_properties() {
    let mut c = Core::new(CoreConfig::default());
    let output = c.handle(
        Timestamp(0),
        Input::Fact {
            name: "surface.kind".into(),
            value: Some(FactValue::Text("window".into())),
        },
    );
    assert!(output.iter().any(|o| matches!(
        o,
        Output::Log {
            level: LogLevel::Warn,
            ..
        }
    )));
    assert!(c.snapshot().facts.is_empty());
}
#[test]
fn surface_limit_is_observed_from_configuration() {
    let mut c = Core::new(CoreConfig {
        max_surfaces: 1,
        ..CoreConfig::default()
    });
    c.handle(Timestamp(0), Input::SurfaceAdded(info(1, true)));
    assert!(
        c.handle(Timestamp(1), Input::SurfaceAdded(info(2, false)))
            .iter()
            .any(|o| matches!(
                o,
                Output::Log {
                    level: LogLevel::Warn,
                    ..
                }
            ))
    );
    assert_eq!(c.snapshot().surfaces.len(), 1);
}
#[test]
fn invalid_thresholds_are_rejected_before_core_construction() {
    assert!(
        CoreConfig {
            tap_slop: Mm(f32::NAN),
            ..CoreConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        CoreConfig {
            backoff_max_ms: 500,
            ..CoreConfig::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        CoreConfig {
            effect_slots: 0,
            ..CoreConfig::default()
        }
        .validate()
        .is_err()
    );
}
