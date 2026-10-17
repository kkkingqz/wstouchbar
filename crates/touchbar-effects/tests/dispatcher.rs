use std::{rc::Rc, sync::Arc, time::Duration};
use touchbar_core::{Input, Output};
use touchbar_effects::*;
use touchbar_model::*;
fn instance(n: u64) -> InstanceId {
    InstanceId {
        number: n,
        generation: Generation(1),
    }
}
fn request(n: u64) -> RequestId {
    RequestId {
        instance: instance(1),
        number: n,
    }
}
fn subscription(n: u64) -> SubscriptionId {
    SubscriptionId {
        instance: instance(n),
        number: n,
    }
}
async fn next(engine: &mut Engine) -> Input {
    futures_lite::future::race(engine.next(), async {
        async_io::Timer::after(Duration::from_secs(3)).await;
        panic!("executor stalled")
    })
    .await
}
fn start(n: u64, effect: Effect, timeout: Option<u64>) -> Output {
    Output::StartEffect {
        request: request(n),
        effect,
        policy: EffectPolicy {
            lifetime: Lifetime::Committed,
            timeout_ms: timeout,
            coalesce: None,
        },
    }
}
#[test]
fn timers_return_to_queue_and_cancel_does_not_emit_late_result() {
    futures_lite::future::block_on(async {
        let (mut engine, _) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        engine.handle(start(1, Effect::TimerAfter { ms: 16 }, None));
        assert!(
            matches!(next(&mut engine).await,Input::EffectDone{request:r,result:Ok(EffectResult::None)}if r==request(1))
        );
        engine.handle(start(2, Effect::TimerAfter { ms: 16 }, None));
        engine.handle(Output::CancelEffect {
            request: request(2),
        });
        engine.handle(start(3, Effect::TimerAfter { ms: 1 }, None));
        assert!(matches!(next(&mut engine).await,Input::EffectDone{request:r,..}if r==request(3)));
    });
}
#[test]
fn identical_subscriptions_share_source_fan_out_and_stop_only_after_last_unsubscribe() {
    futures_lite::future::block_on(async {
        let (mut engine, control) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        let kind = SubscriptionKind::MediaWatch;
        for n in [1, 2] {
            engine.handle(Output::Subscribe {
                subscription: subscription(n),
                kind: kind.clone(),
            });
        }
        assert_eq!(engine.source_count(), 1);
        control.emit(&kind, SubscriptionData::Tick).unwrap();
        for n in [1, 2] {
            assert!(
                matches!(next(&mut engine).await,Input::SubscriptionData{subscription:s,data:SubscriptionData::Tick}if s==subscription(n))
            );
        }
        engine.handle(Output::Unsubscribe {
            subscription: subscription(1),
        });
        assert_eq!(engine.source_count(), 1);
        control.lose(&kind, EffectError::Unavailable).unwrap();
        assert!(
            matches!(next(&mut engine).await,Input::SubscriptionLost{subscription:s,..}if s==subscription(2))
        );
        assert_eq!(engine.source_count(), 0);
        for n in [3, 4] {
            engine.handle(Output::Subscribe {
                subscription: subscription(n),
                kind: kind.clone(),
            });
        }
        engine.handle(Output::Unsubscribe {
            subscription: subscription(3),
        });
        assert_eq!(engine.source_count(), 1);
        engine.handle(Output::Unsubscribe {
            subscription: subscription(4),
        });
        assert_eq!(engine.source_count(), 0);
        assert_eq!(control.sources(), 0);
    });
}
#[test]
fn desktop_facts_replay_current_value_and_interval_has_a_minimum() {
    futures_lite::future::block_on(async {
        let (mut engine, _) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        engine.fact("app.id".into(), Some(FactValue::Text("first".into())));
        engine.handle(Output::Subscribe {
            subscription: subscription(1),
            kind: SubscriptionKind::DesktopFact {
                name: "app.id".into(),
            },
        });
        assert!(
            matches!(next(&mut engine).await,Input::SubscriptionData{data:SubscriptionData::Fact(Some(FactValue::Text(s))),..}if s=="first")
        );
        engine.fact("app.id".into(), None);
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionData {
                data: SubscriptionData::Fact(None),
                ..
            }
        ));
        engine.handle(Output::Subscribe {
            subscription: subscription(2),
            kind: SubscriptionKind::TimerInterval { ms: 1 },
        });
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionLost { .. }
        ));
        engine.handle(Output::Subscribe {
            subscription: subscription(3),
            kind: SubscriptionKind::TimerInterval { ms: 16 },
        });
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionData {
                data: SubscriptionData::Tick,
                ..
            }
        ));
    });
}
#[test]
fn fake_rules_timeout_committed_lifetime_and_audio_state_are_supported() {
    futures_lite::future::block_on(async {
        let (mut engine, control) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        control.set_rule(
            "spawn.run",
            FakeRule {
                delay_ms: 100,
                result: Ok(EffectResult::None),
            },
        );
        engine.handle(start(
            1,
            Effect::SpawnRun(SpawnSpec::new(vec!["unused".into()])),
            Some(10),
        ));
        assert!(matches!(
            next(&mut engine).await,
            Input::EffectDone {
                result: Err(EffectError::Timeout),
                ..
            }
        ));
        engine.handle(start(2, Effect::TimerAfter { ms: 16 }, None));
        assert!(!engine.handle(Output::DisposeInstance {
            instance: instance(1)
        }));
        assert!(
            matches!(next(&mut engine).await,Input::EffectDone{request:r,result:Ok(_)}if r==request(2))
        );
        engine.handle(Output::Subscribe {
            subscription: subscription(4),
            kind: SubscriptionKind::AudioWatch {
                target: AudioTarget::DefaultSink,
            },
        });
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionData {
                data: SubscriptionData::Audio(_),
                ..
            }
        ));
        engine.handle(start(
            3,
            Effect::AudioSetMute {
                target: AudioTarget::DefaultSink,
                value: MuteValue::Toggle,
            },
            None,
        ));
        assert!(matches!(
            next(&mut engine).await,
            Input::EffectDone {
                result: Ok(EffectResult::Mute(true)),
                ..
            }
        ));
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionData {
                data: SubscriptionData::Audio(AudioState { muted: true, .. }),
                ..
            }
        ));
    });
}
#[test]
fn native_dispatches_desktop_and_reports_unsupported_modules() {
    futures_lite::future::block_on(async {
        let backend = touchbar_desktop::FakeBackend::default();
        let (tx, _rx) = async_channel::bounded(32);
        let mut started = backend.clone();
        touchbar_desktop::DesktopBackend::start(&mut started, tx).unwrap();
        let mut engine = Engine::native(
            Config::new(env!("CARGO_BIN_EXE_touchbar-spawn-helper").into()),
            Rc::new(backend.clone()),
            Arc::new(touchbar_audio::UnavailableAudio),
        );
        engine.handle(start(1, Effect::DesktopBrightness { level: 0.4 }, None));
        assert!(matches!(
            next(&mut engine).await,
            Input::EffectDone { result: Ok(_), .. }
        ));
        assert_eq!(
            backend.fact(&"display.brightness".into()),
            Some(FactValue::Number(0.4))
        );
        for (n, effect) in [
            (2, Effect::MediaNext),
            (
                3,
                Effect::OpenUri {
                    uri: "https://example.org".into(),
                },
            ),
        ] {
            engine.handle(start(n, effect, None));
            assert!(matches!(
                next(&mut engine).await,
                Input::EffectDone {
                    result: Err(EffectError::Unsupported),
                    ..
                }
            ));
        }
    });
}
#[test]
fn a_busy_source_does_not_starve_other_sources() {
    futures_lite::future::block_on(async {
        let (mut engine, control) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        let a = SubscriptionKind::MediaWatch;
        let b = SubscriptionKind::DbusSignal(DbusMatch {
            bus: Bus::Session,
            sender: None,
            path: None,
            interface: None,
            member: None,
        });
        for (n, kind) in [(1, a.clone()), (2, b.clone())] {
            engine.handle(Output::Subscribe {
                subscription: subscription(n),
                kind,
            });
        }
        for _ in 0..10 {
            control.emit(&a, SubscriptionData::Tick).unwrap();
        }
        control.emit(&b, SubscriptionData::Tick).unwrap();
        assert!(
            matches!(next(&mut engine).await,Input::SubscriptionData{subscription:s,..}if s==subscription(1))
        );
        assert!(
            matches!(next(&mut engine).await,Input::SubscriptionData{subscription:s,..}if s==subscription(2))
        );
    });
}
#[test]
fn late_subscriber_gets_cached_audio_state_without_restarting_source() {
    futures_lite::future::block_on(async {
        let (mut engine, control) = Engine::fake(Rc::new(touchbar_desktop::NullBackend::default()));
        let kind = SubscriptionKind::AudioWatch {
            target: AudioTarget::DefaultSink,
        };
        engine.handle(Output::Subscribe {
            subscription: subscription(1),
            kind: kind.clone(),
        });
        assert!(matches!(
            next(&mut engine).await,
            Input::SubscriptionData {
                data: SubscriptionData::Audio(_),
                ..
            }
        ));
        engine.handle(Output::Subscribe {
            subscription: subscription(2),
            kind,
        });
        assert_eq!(engine.source_count(), 1);
        assert_eq!(control.sources(), 1);
        assert!(
            matches!(next(&mut engine).await,Input::SubscriptionData{subscription:s,data:SubscriptionData::Audio(_)}if s==subscription(2))
        );
    });
}
fn native_engine() -> (Engine, touchbar_effects::spawn::SpawnPool) {
    let config = Config::new(env!("CARGO_BIN_EXE_touchbar-spawn-helper").into());
    let pool = config.spawn.clone();
    (
        Engine::native(
            config,
            Rc::new(touchbar_desktop::NullBackend::default()),
            Arc::new(touchbar_audio::UnavailableAudio),
        ),
        pool,
    )
}
#[test]
fn native_spawn_action_and_shared_lines_return_through_dispatcher() {
    futures_lite::future::block_on(async {
        let (mut engine, _) = native_engine();
        let spec = SpawnSpec::new(vec![
            env!("CARGO_BIN_EXE_touchbar-effects-test-helper").into(),
            "lines".into(),
        ]);
        engine.handle(start(1, Effect::SpawnRun(spec.clone()), Some(3000)));
        match next(&mut engine).await {
            Input::EffectDone {
                result: Ok(EffectResult::Spawn(result)),
                ..
            } => {
                assert_eq!(result.status, 0);
                assert_eq!(result.stdout, "one\ntwo\r\ntail");
            }
            other => panic!("{other:?}"),
        }
        for n in [1, 2] {
            engine.handle(Output::Subscribe {
                subscription: subscription(n),
                kind: SubscriptionKind::SpawnLines(spec.clone()),
            });
        }
        assert_eq!(engine.source_count(), 1);
        let mut lines = vec![];
        let mut lost = vec![];
        while lost.len() < 2 {
            match next(&mut engine).await {
                Input::SubscriptionData {
                    subscription,
                    data: SubscriptionData::Line(line),
                } => lines.push((subscription, line)),
                Input::SubscriptionLost { subscription, .. } => lost.push(subscription),
                other => panic!("{other:?}"),
            }
        }
        for n in [1, 2] {
            assert_eq!(
                lines
                    .iter()
                    .filter(|(s, _)| *s == subscription(n))
                    .map(|(_, s)| s.as_str())
                    .collect::<Vec<_>>(),
                ["one", "two", "tail"]
            );
        }
        assert_eq!(engine.source_count(), 0);
    });
}
#[test]
fn last_shared_unsubscribe_cancels_native_source() {
    futures_lite::future::block_on(async {
        let (mut engine, pool) = native_engine();
        let kind = SubscriptionKind::SpawnLines(SpawnSpec::new(vec![
            env!("CARGO_BIN_EXE_touchbar-effects-test-helper").into(),
            "sleep".into(),
        ]));
        for n in [1, 2] {
            engine.handle(Output::Subscribe {
                subscription: subscription(n),
                kind: kind.clone(),
            });
        }
        futures_lite::future::race(
            async {
                let event = engine.next().await;
                panic!("unexpected {event:?}")
            },
            async {
                async_io::Timer::after(Duration::from_millis(30)).await;
            },
        )
        .await;
        assert_eq!(pool.active(), 1);
        engine.handle(Output::Unsubscribe {
            subscription: subscription(1),
        });
        assert_eq!(pool.active(), 1);
        engine.handle(Output::Unsubscribe {
            subscription: subscription(2),
        });
        assert_eq!(engine.source_count(), 0);
        let end = std::time::Instant::now() + Duration::from_secs(2);
        while pool.active() != 0 {
            assert!(std::time::Instant::now() < end, "source group not reaped");
            async_io::Timer::after(Duration::from_millis(5)).await;
        }
    });
}
