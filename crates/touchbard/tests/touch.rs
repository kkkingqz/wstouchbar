use touchbar_model::TouchPhase::{self, Cancel, Down, Move, Up};
use touchbard::input::{Axis, AxisTransform, Geometry, InputEvent, RawSlot, Step, TouchState};
fn geometry(transform: AxisTransform) -> Geometry {
    Geometry::new(
        2008,
        60,
        Axis {
            minimum: 0,
            maximum: 2000,
        },
        Axis {
            minimum: 0,
            maximum: 60,
        },
        transform,
    )
    .unwrap()
}
fn event(name: &str, value: i32) -> InputEvent {
    let (kind, code) = match name {
        "SLOT" => (3, 0x2f),
        "TRACK" => (3, 0x39),
        "X" => (3, 0x35),
        "Y" => (3, 0x36),
        "SYN" => (0, 0),
        "DROP" => (0, 3),
        _ => panic!("fixture event"),
    };
    InputEvent { kind, code, value }
}
fn replay(state: &mut TouchState, fixture: &str) -> (Vec<(u32, TouchPhase, i32, i32)>, bool) {
    let mut out = Vec::new();
    let mut resync = false;
    for line in fixture.lines().filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(' ').unwrap();
        match state.handle(event(name, value.parse().unwrap())).unwrap() {
            Step::Events(events) => {
                out.extend(events.into_iter().map(|t| (t.tracking, t.phase, t.x, t.y)))
            }
            Step::Resync => resync = true,
        }
    }
    (out, resync)
}
#[test]
fn single_two_and_reused_zero_from_text_fixtures() {
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    assert_eq!(
        replay(&mut s, include_str!("fixtures/touch/single.txt")),
        (
            vec![(0, Down, 0, 0), (0, Move, 2007, 59), (0, Up, 2007, 59)],
            false
        )
    );
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    assert_eq!(
        replay(&mut s, include_str!("fixtures/touch/two.txt")),
        (
            vec![
                (0, Down, 0, 0),
                (7, Down, 2007, 59),
                (7, Move, 1004, 30),
                (0, Up, 0, 0),
                (7, Up, 1004, 30)
            ],
            false
        )
    );
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    assert_eq!(
        replay(&mut s, include_str!("fixtures/touch/reuse-zero.txt")),
        (
            vec![(0, Down, 0, 0), (0, Up, 0, 0), (0, Down, 2007, 59)],
            false
        )
    );
}
#[test]
fn dropped_stream_ignores_unreliable_events_and_reconciles_snapshot_by_tracking_id() {
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    let (out, resync) = replay(&mut s, include_str!("fixtures/touch/dropped.txt"));
    assert!(resync);
    assert_eq!(out, vec![(0, Down, 0, 0), (7, Down, 2007, 59)]);
    let out = s
        .restore(
            &[
                RawSlot {
                    tracking: Some(9),
                    x: 0,
                    y: 0,
                },
                RawSlot {
                    tracking: Some(7),
                    x: 1000,
                    y: 30,
                },
            ],
            1,
        )
        .unwrap();
    assert_eq!(
        out.iter()
            .map(|t| (t.tracking, t.phase, t.x, t.y))
            .collect::<Vec<_>>(),
        vec![(0, Cancel, 0, 0), (7, Move, 1004, 30), (9, Down, 0, 0)]
    );
    assert_eq!(
        replay(&mut s, "X 2000\nSYN 0"),
        (vec![(7, Move, 2007, 30)], false)
    );
    let cancelled = s.cancel_all();
    assert_eq!(cancelled.len(), 2);
    assert!(cancelled.iter().all(|t| t.phase == Cancel));
    assert!(s.cancel_all().is_empty());
}
#[test]
fn coordinates_are_clamped_and_axis_swap_is_explicit_not_inferred_from_drm() {
    let g = geometry(AxisTransform::Identity);
    assert_eq!(g.point(-500, 900), (0, 59));
    let g = Geometry::new(
        2008,
        60,
        Axis {
            minimum: -30,
            maximum: 30,
        },
        Axis {
            minimum: -1000,
            maximum: 1000,
        },
        AxisTransform::Swap,
    )
    .unwrap();
    assert_eq!(g.point(-30, 1000), (2007, 0));
    assert_eq!(g.point(30, -1000), (0, 59));
    assert!(
        Geometry::new(
            0,
            60,
            Axis {
                minimum: 0,
                maximum: 1
            },
            Axis {
                minimum: 0,
                maximum: 1
            },
            AxisTransform::Identity
        )
        .is_err()
    );
}
#[test]
fn rapid_edges_before_syn_are_not_lost_and_invalid_snapshot_is_atomic() {
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    assert_eq!(
        replay(
            &mut s,
            "TRACK 0\nX 1000\nY 30\nTRACK -1\nTRACK 0\nX 2000\nSYN 0"
        ),
        (
            vec![(0, Down, 1004, 30), (0, Up, 1004, 30), (0, Down, 2007, 30)],
            false
        )
    );
    let duplicate = RawSlot {
        tracking: Some(7),
        x: 0,
        y: 0,
    };
    assert!(s.restore(&[duplicate, duplicate], 0).is_err());
    let cancel = s.cancel_all();
    assert_eq!(cancel.len(), 1);
    assert_eq!(cancel[0].tracking, 0);
}

proptest::proptest! {
    #[test]
    fn every_emitted_touch_has_balanced_edges_and_bounded_coordinates(sequence in proptest::collection::vec((0u8..7,-2i32..8),0..160)) {
        use std::collections::BTreeSet;
        let mut state=TouchState::new(geometry(AxisTransform::Identity),2,0).unwrap();
        let mut active=BTreeSet::new();
        for (op,value) in sequence {
            let name=match op {0=>"SLOT",1=>"TRACK",2=>"X",3=>"Y",4=>"DROP",_=>"SYN"};
            let events=match state.handle(event(name,value)) {
                Ok(Step::Events(events))=>events,
                Ok(Step::Resync)=>state.restore(&[RawSlot {tracking:None,x:0,y:0};2],0).unwrap(),
                Err(_)=> {let events=state.cancel_all();state=TouchState::new(geometry(AxisTransform::Identity),2,0).unwrap();events},
            };
            for touch in events {
                proptest::prop_assert!((0..2008).contains(&touch.x) && (0..60).contains(&touch.y));
                match touch.phase {
                    Down=>proptest::prop_assert!(active.insert(touch.tracking)),
                    Move=>proptest::prop_assert!(active.contains(&touch.tracking)),
                    Up|Cancel=>proptest::prop_assert!(active.remove(&touch.tracking)),
                }
            }
        }
        for touch in state.cancel_all() {proptest::prop_assert_eq!(touch.phase,Cancel);proptest::prop_assert!(active.remove(&touch.tracking));}
        proptest::prop_assert!(active.is_empty());
    }
}

#[test]
fn resync_preserves_contact_that_moved_to_another_slot() {
    let mut s = TouchState::new(geometry(AxisTransform::Identity), 2, 0).unwrap();
    replay(
        &mut s,
        "SLOT 1\nTRACK 7\nX 2000\nY 60\nSYN 0\nDROP 0\nSYN 0",
    );
    let out = s
        .restore(
            &[
                RawSlot {
                    tracking: Some(7),
                    x: 0,
                    y: 0,
                },
                RawSlot {
                    tracking: None,
                    x: 0,
                    y: 0,
                },
            ],
            0,
        )
        .unwrap();
    assert_eq!(
        out.iter()
            .map(|t| (t.tracking, t.phase, t.x, t.y))
            .collect::<Vec<_>>(),
        vec![(7, Move, 0, 0)]
    );
}
