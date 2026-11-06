//! Independent source availability and per-surface physical diagnostics.
use crate::{ControlOptions, control::format_status_with_daemon};
use futures_lite::{
    future::{or, zip},
    io::AsyncWriteExt,
};
use serde::Serialize;
use serde_json::Value;
use std::{fmt::Write as _, io, path::Path, time::Duration};
use touchbar_protocol::control as daemon;

pub(crate) async fn daemon_call(
    override_path: Option<&Path>,
    command: daemon::Command,
) -> Result<daemon::Reply, String> {
    daemon_call_with_timeout(override_path, command, Duration::from_secs(10)).await
}
fn trusted_peer(peer: u32, caller: u32, explicit: bool) -> bool {
    peer == 0 || explicit && peer == caller
}
async fn daemon_call_with_timeout(
    override_path: Option<&Path>,
    command: daemon::Command,
    timeout: Duration,
) -> Result<daemon::Reply, String> {
    let path = override_path.unwrap_or(Path::new("/run/touchbar/control.sock"));
    let operation = async {
        let mut stream = async_io::Async::<std::os::unix::net::UnixStream>::connect(path).await?;
        let uid = rustix::net::sockopt::socket_peercred(stream.get_ref())?.uid;
        // The normal endpoint is root-owned. An explicitly chosen development
        // endpoint may also belong to the caller; other users are never trusted.
        if !trusted_peer(
            uid.as_raw(),
            rustix::process::getuid().as_raw(),
            override_path.is_some(),
        ) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "unexpected touchbard control peer UID",
            ));
        }
        stream
            .write_all(&daemon::encode(&daemon::Request::new(command))?)
            .await?;
        let response: daemon::Response =
            touchbar_control::read_async_with_limit(&mut stream, daemon::MAX_FRAME).await?;
        response.validate()?;
        Ok(response.reply)
    };
    or(operation, async {
        async_io::Timer::after(timeout).await;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "touchbard control timeout",
        ))
    })
    .await
    .map_err(|e| e.to_string())
}
#[derive(Serialize)]
struct Source {
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}
impl Source {
    fn new(result: Result<Value, String>) -> Self {
        match result {
            Ok(status) => Self {
                available: true,
                status: Some(status),
                error: None,
            },
            Err(error) => Self {
                available: false,
                status: None,
                error: Some(error),
            },
        }
    }
}
#[derive(Serialize)]
struct Surface {
    name: String,
    session: Option<Value>,
    daemon: Option<Value>,
    transport: Option<Value>,
    instances: Vec<Value>,
    last_calls: Vec<Value>,
    requests: Vec<Value>,
    subscriptions: Vec<Value>,
}
#[derive(Serialize)]
struct Status {
    session: Source,
    daemon: Source,
    surfaces: Vec<Surface>,
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn session_status(value: Value) -> Result<Value, String> {
    serde_json::from_value::<touchbar_core::CoreSnapshot>(value["core"].clone())
        .map_err(|e| e.to_string())?;
    serde_json::from_value::<Vec<(touchbar_model::InstanceId, touchbar_model::Timestamp)>>(
        value["last_calls"].clone(),
    )
    .map_err(|e| e.to_string())?;
    Ok(value)
}
impl Status {
    fn new(session: Result<Value, String>, daemon: Result<Value, String>) -> Self {
        let session = Source::new(session.and_then(session_status));
        let daemon = Source::new(daemon);
        let mut surfaces = Vec::new();
        if let Some(s) = &session.status {
            for surface in array(&s["core"]["surfaces"]) {
                let id = &surface["info"]["id"];
                let physical = surface["info"]["kind"] == "touchbar";
                let instances = array(&s["core"]["instances"])
                    .iter()
                    .filter(|i| i["surface"] == *id)
                    .cloned()
                    .collect::<Vec<_>>();
                let last_calls = array(&s["last_calls"])
                    .iter()
                    .filter(|c| instances.iter().any(|i| i["id"] == c[0]))
                    .cloned()
                    .collect();
                let requests = array(&s["core"]["requests"])
                    .iter()
                    .filter(|r| r["surface"] == *id)
                    .cloned()
                    .collect();
                let subscriptions = array(&s["core"]["subscriptions"])
                    .iter()
                    .filter(|r| instances.iter().any(|i| i["id"] == r["owner"]))
                    .cloned()
                    .collect();
                surfaces.push(Surface {
                    name: surface["info"]["name"]
                        .as_str()
                        .expect("validated surface name")
                        .into(),
                    session: Some(surface.clone()),
                    daemon: physical.then(|| daemon.status.clone()).flatten(),
                    transport: physical
                        .then(|| s.get("touchbar").filter(|v| !v.is_null()).cloned())
                        .flatten(),
                    instances,
                    last_calls,
                    requests,
                    subscriptions,
                });
            }
        }
        if daemon.available && !surfaces.iter().any(|s| s.name == "touchbar") {
            surfaces.push(Surface {
                name: "touchbar".into(),
                session: None,
                daemon: daemon.status.clone(),
                transport: None,
                instances: vec![],
                last_calls: vec![],
                requests: vec![],
                subscriptions: vec![],
            });
        }
        Self {
            session,
            daemon,
            surfaces,
        }
    }
    fn text(&self) -> Result<String, String> {
        let mut text = String::new();
        if let Some(error) = &self.session.error {
            writeln!(text, "Session unavailable: {error}").unwrap();
        }
        if let Some(error) = &self.daemon.error {
            writeln!(text, "Daemon unavailable: {error}").unwrap();
        }
        if let Some(status) = &self.session.status {
            text.push_str(&format_status_with_daemon(
                status,
                self.daemon.status.as_ref(),
            )?);
        }
        if self
            .surfaces
            .iter()
            .any(|s| s.name == "touchbar" && s.session.is_none())
        {
            writeln!(text, "\ntouchbar (session surface unavailable):").unwrap();
            format_daemon(
                &mut text,
                self.daemon.status.as_ref().expect("daemon-only surface"),
            );
        }
        Ok(text)
    }
}
pub(crate) fn format_daemon(text: &mut String, daemon: &Value) {
    writeln!(text, "  Daemon mode: {}; reason: {}\n  Owner: {}\n  Recovery: {}\n  Backlight: {}; Fn bridge: {}\n  Device: {}\n  Driver errors: {}; inhibitor error: {}",
        daemon["mode"], daemon["reason"], daemon["owner"], daemon["recovery"], daemon["backlight"], daemon["fn_bridge"],
        daemon["device"], daemon["drivers"]["errors"], daemon["inhibitor_error"]).unwrap();
}
pub(crate) fn format_transport(text: &mut String, transport: &Value) {
    writeln!(
        text,
        "  Connection: {}; buffers: {}; error: {}",
        transport["connected"], transport["path"], transport["error"]
    )
    .unwrap();
    for (metric, unit) in [("commit_us", "us"), ("ack_ms", "ms")] {
        let v = &transport[metric];
        writeln!(
            text,
            "  {metric}: n={}, p50={} {unit}, p99={} {unit}",
            v["samples"], v["p50"], v["p99"]
        )
        .unwrap();
    }
}
pub(crate) fn run_status(options: &ControlOptions) -> Result<String, String> {
    let session = async {
        let runtime = touchbar_control::runtime_dir().map_err(|e| e.to_string())?;
        match touchbar_control::call(
            &touchbar_control::socket_path(&runtime, false),
            touchbar_control::Command::Status,
        )
        .await
        .map_err(|e| e.to_string())?
        {
            touchbar_control::Reply::Status(status) => Ok(status),
            touchbar_control::Reply::Error(e) => Err(e),
            _ => Err("unexpected session status response".into()),
        }
    };
    let daemon = async {
        match daemon_call(options.daemon_socket.as_deref(), daemon::Command::Status).await? {
            daemon::Reply::Status(status)
                if status.is_object()
                    && status["recovery"].is_boolean()
                    && !status["mode"].is_null() =>
            {
                Ok(status)
            }
            daemon::Reply::Error(e) => Err(e),
            _ => Err("unexpected daemon status response".into()),
        }
    };
    let (session, daemon) = futures_lite::future::block_on(zip(session, daemon));
    let status = Status::new(session, daemon);
    if options.json {
        serde_json::to_string_pretty(&status)
            .map(|s| s + "\n")
            .map_err(|e| e.to_string())
    } else {
        status.text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_lite::io::AsyncReadExt;
    #[test]
    fn default_endpoint_requires_root_and_explicit_endpoint_never_trusts_another_user() {
        let caller = 1000;
        assert!(trusted_peer(0, caller, false));
        assert!(!trusted_peer(caller, caller, false));
        assert!(trusted_peer(caller, caller, true));
        assert!(!trusted_peer(1001, caller, true));
    }
    #[test]
    fn silent_daemon_times_out_and_drops_the_connection() {
        futures_lite::future::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("control.sock");
            let listener =
                async_io::Async::<std::os::unix::net::UnixListener>::bind(&path).unwrap();
            let server = async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _: daemon::Request =
                    touchbar_control::read_async_with_limit(&mut stream, daemon::MAX_FRAME)
                        .await
                        .unwrap();
                // The independent server deadline prevents a missing client timer
                // from hanging the test; that path yields EOF instead of timeout.
                or(
                    async {
                        let mut byte = [0];
                        assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
                    },
                    async {
                        async_io::Timer::after(Duration::from_secs(1)).await;
                    },
                )
                .await;
            };
            let request = daemon_call_with_timeout(
                Some(&path),
                daemon::Command::Status,
                Duration::from_millis(30),
            );
            let (_, result) = zip(server, request).await;
            assert!(result.unwrap_err().contains("timeout"));
        });
    }
    #[test]
    fn daemon_response_version_and_frame_budget_are_checked() {
        futures_lite::future::block_on(async {
            for response in [
                touchbar_control::encode(&serde_json::json!({"version":99,"reply":{"kind":"ack"}}))
                    .unwrap(),
                (u32::try_from(daemon::MAX_FRAME).unwrap() + 1)
                    .to_le_bytes()
                    .to_vec(),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("control.sock");
                let listener =
                    async_io::Async::<std::os::unix::net::UnixListener>::bind(&path).unwrap();
                let server = async {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let _: daemon::Request =
                        touchbar_control::read_async_with_limit(&mut stream, daemon::MAX_FRAME)
                            .await
                            .unwrap();
                    stream.write_all(&response).await.unwrap();
                };
                let (_, result) =
                    zip(server, daemon_call(Some(&path), daemon::Command::Status)).await;
                assert!(result.is_err(), "invalid response accepted");
            }
        });
    }
}
