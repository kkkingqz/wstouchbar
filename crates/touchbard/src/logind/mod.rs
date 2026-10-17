//! logind facts and a cancellation-safe system-bus monitor. No PID/session guesses.
use crate::Session;
use std::{collections::BTreeMap, io};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Structure, Value};
pub type Properties = BTreeMap<String, OwnedValue>;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRef {
    pub id: String,
    pub path: OwnedObjectPath,
}
fn invalid(message: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}
fn prop<'a>(properties: &'a Properties, key: &str) -> io::Result<&'a OwnedValue> {
    properties
        .get(key)
        .ok_or_else(|| invalid(format!("missing logind property {key}")))
}
fn string(properties: &Properties, key: &str) -> io::Result<String> {
    <&str>::try_from(prop(properties, key)?)
        .map(str::to_owned)
        .map_err(invalid)
}
fn boolean(properties: &Properties, key: &str) -> io::Result<bool> {
    bool::try_from(prop(properties, key)?).map_err(invalid)
}
fn fields(value: &OwnedValue) -> io::Result<&[Value<'_>]> {
    let structure = <&Structure>::try_from(value).map_err(invalid)?;
    if structure.fields().len() != 2 {
        return Err(invalid("invalid logind reference"));
    }
    Ok(structure.fields())
}
fn path(value: &Value<'_>) -> io::Result<OwnedObjectPath> {
    <&ObjectPath>::try_from(value)
        .map_err(invalid)?
        .as_str()
        .try_into()
        .map_err(invalid)
}
pub(crate) fn session_ref(value: &OwnedValue) -> io::Result<SessionRef> {
    let f = fields(value)?;
    let id = <&str>::try_from(&f[0]).map_err(invalid)?.to_owned();
    if id.len() > 128 {
        return Err(invalid("logind session id too long"));
    }
    Ok(SessionRef {
        id,
        path: path(&f[1])?,
    })
}
pub(crate) fn user_ref(properties: &Properties) -> io::Result<(u32, OwnedObjectPath)> {
    let f = fields(prop(properties, "User")?)?;
    Ok((u32::try_from(&f[0]).map_err(invalid)?, path(&f[1])?))
}
/// User.Display must name this exact active seat0 session and belong to the
/// same UID. Keep ineligible greeter/tty facts for diagnostics; H4 owns policy.
pub fn session_fact(
    active: &SessionRef,
    properties: &Properties,
    user: &Properties,
) -> io::Result<Option<Session>> {
    if active.id.is_empty() || active.path.as_str() == "/" {
        return Ok(None);
    }
    if string(properties, "Id")? != active.id {
        return Err(invalid("logind session id disagrees with seat0"));
    }
    let (uid, _) = user_ref(properties)?;
    let display_uid = u32::try_from(prop(user, "UID")?).map_err(invalid)?;
    let display = session_ref(prop(user, "Display")?)?;
    let user_display =
        (display_uid == uid && display.path == active.path && !display.id.is_empty())
            .then_some(display.id);
    Ok(Some(Session {
        uid,
        id: active.id.clone(),
        user_display,
        active: boolean(properties, "Active")?,
        state: string(properties, "State")?,
        class: string(properties, "Class")?,
        kind: string(properties, "Type")?,
        locked: boolean(properties, "LockedHint")?,
    }))
}
mod driver;
pub use driver::{Monitor, Notice};
