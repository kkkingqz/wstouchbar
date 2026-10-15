use touchbar_audio::model::*;
use touchbar_model::*;
fn volume(value: f32, muted: bool) -> Volume {
    Volume {
        channels: vec![value, value],
        muted,
    }
}
fn node(name: &str, device: Option<(u32, i32)>) -> Node {
    Node {
        name: name.into(),
        description: name.into(),
        device,
        volume: Some(volume(1.0, false)),
    }
}
#[test]
fn cubic_scale_and_validation() {
    assert!((from_linear(0.125) - 0.5).abs() < 1e-9);
    assert_eq!(to_linear(0.5).unwrap(), 0.125);
    assert_eq!(to_linear(1.5).unwrap(), 3.375);
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.5001] {
        assert!(to_linear(invalid).is_err());
    }
    assert_eq!(from_linear(-1.0), 0.0);
    assert_eq!(from_linear(f32::NAN), 0.0);
}
#[test]
fn defaults_follow_metadata_and_removal() {
    let mut graph = Graph::default();
    assert!(!graph.state(AudioTarget::DefaultSink).available);
    graph.nodes.insert(10, node("a", None));
    graph.nodes.insert(20, node("b", None));
    graph.metadata("default.audio.sink", Some(r#"{"name":"a"}"#));
    assert_eq!(graph.state(AudioTarget::DefaultSink).name, "a");
    graph.metadata("default.audio.sink", Some(r#"{"name":"b"}"#));
    assert_eq!(
        graph.control(AudioTarget::DefaultSink).unwrap().0,
        Control::Node(20)
    );
    graph.metadata("default.audio.source", Some(r#"{"name":"a"}"#));
    assert_eq!(graph.state(AudioTarget::DefaultSource).name, "a");
    graph.remove(20);
    assert!(!graph.state(AudioTarget::DefaultSink).available);
    graph.metadata("default.audio.source", None);
    assert!(!graph.state(AudioTarget::DefaultSource).available);
}
#[test]
fn matching_route_has_priority_and_falls_back_to_props() {
    let mut graph = Graph::default();
    graph.nodes.insert(10, node("a", Some((50, 2))));
    graph.metadata("default.audio.sink", Some(r#"{"name":"a"}"#));
    assert_eq!(
        graph.control(AudioTarget::DefaultSink).unwrap().0,
        Control::Node(10)
    );
    graph.routes.insert(
        (50, 0),
        Route {
            profile_device: 1,
            volume: volume(0.008, true),
        },
    );
    graph.routes.insert(
        (50, 3),
        Route {
            profile_device: 2,
            volume: volume(0.125, true),
        },
    );
    assert_eq!(
        graph.control(AudioTarget::DefaultSink).unwrap().0,
        Control::Route {
            device: 50,
            index: 3,
            profile_device: 2
        }
    );
    let state = graph.state(AudioTarget::DefaultSink);
    assert!((state.volume - 0.5).abs() < 1e-6);
    assert!(state.muted);
    graph.remove(50);
    assert_eq!(
        graph.control(AudioTarget::DefaultSink).unwrap().0,
        Control::Node(10)
    );
}
#[test]
fn metadata_clear_and_invalid_data_do_not_retain_stale_defaults() {
    let mut graph = Graph::default();
    graph.nodes.insert(1, node("a", None));
    graph.metadata("default.audio.sink", Some(r#"{"name":"a"}"#));
    graph.metadata("default.audio.sink", Some("bad json"));
    assert!(!graph.state(AudioTarget::DefaultSink).available);
}
