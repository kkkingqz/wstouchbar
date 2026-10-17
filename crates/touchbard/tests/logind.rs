use std::collections::BTreeMap;
use touchbard::logind::{Properties, SessionRef, session_fact};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
fn string(s: &str) -> OwnedValue {
    Value::from(s).try_into().unwrap()
}
fn reference(id: &str, path: &str) -> OwnedValue {
    Value::from((id, ObjectPath::try_from(path).unwrap()))
        .try_into()
        .unwrap()
}
fn fixture() -> (SessionRef, Properties, Properties) {
    let seat = SessionRef {
        id: "s2".into(),
        path: "/org/freedesktop/login1/session/s2".try_into().unwrap(),
    };
    let session = BTreeMap::from([
        ("Id".into(), string("s2")),
        (
            "User".into(),
            Value::from((
                1000u32,
                ObjectPath::try_from("/org/freedesktop/login1/user/_1000").unwrap(),
            ))
            .try_into()
            .unwrap(),
        ),
        ("Active".into(), true.into()),
        ("State".into(), string("active")),
        ("Class".into(), string("user")),
        ("Type".into(), string("wayland")),
        ("LockedHint".into(), false.into()),
    ]);
    let user = BTreeMap::from([
        ("UID".into(), 1000u32.into()),
        ("Display".into(), reference("s2", seat.path.as_str())),
    ]);
    (seat, session, user)
}
#[test]
fn owner_fact_requires_graphical_active_session_and_matching_user_display() {
    let (seat, mut session, mut user) = fixture();
    let s = session_fact(&seat, &session, &user).unwrap().unwrap();
    assert!(s.eligible(1000));
    assert!(!s.eligible(1001));
    session.insert("LockedHint".into(), true.into());
    let s = session_fact(&seat, &session, &user).unwrap().unwrap();
    assert!(s.locked);
    assert!(s.eligible(1000));
    user.insert("UID".into(), 1001u32.into());
    assert!(
        !session_fact(&seat, &session, &user)
            .unwrap()
            .unwrap()
            .eligible(1000)
    );
    let (seat, mut session, user) = fixture();
    for (key, value) in [("Class", "greeter"), ("Type", "tty"), ("State", "closing")] {
        let original = session.insert(key.into(), string(value)).unwrap();
        assert!(
            !session_fact(&seat, &session, &user)
                .unwrap()
                .unwrap()
                .eligible(1000)
        );
        session.insert(key.into(), original);
    }
}
#[test]
fn absent_or_malformed_security_properties_fail_closed() {
    let (seat, mut session, mut user) = fixture();
    session.remove("Active");
    assert!(session_fact(&seat, &session, &user).is_err());
    let (seat, session, _) = fixture();
    user.insert(
        "Display".into(),
        reference("s2", "/org/freedesktop/login1/session/other"),
    );
    assert!(
        !session_fact(&seat, &session, &user)
            .unwrap()
            .unwrap()
            .eligible(1000)
    );
    let (mut seat, session, user) = fixture();
    seat.id.clear();
    seat.path = "/".try_into().unwrap();
    assert_eq!(session_fact(&seat, &session, &user).unwrap(), None);
}
