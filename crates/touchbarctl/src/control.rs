use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};
use touchbar_control::{Command, Reply, Theme};
use touchbar_model::{FactValue, Scalar};
use touchbar_protocol::{Mode, control as daemon};

#[derive(Clone, Debug, PartialEq)]
pub enum ControlCommand {
    Status,
    Session(Command),
    Daemon(daemon::Command),
}
#[derive(Debug)]
pub struct ControlOptions {
    pub simulator: bool,
    pub json: bool,
    pub output: Option<PathBuf>,
    pub command: ControlCommand,
    pub daemon_socket: Option<PathBuf>,
}
impl ControlOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut simulator = false;
        let mut daemon_socket = None;
        let mut rest = Vec::new();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--simulator" => {
                    if simulator {
                        return Err("duplicate --simulator".into());
                    }
                    simulator = true;
                }
                "--daemon-socket" => {
                    if daemon_socket.is_some() {
                        return Err("duplicate --daemon-socket".into());
                    }
                    let path = PathBuf::from(args.next().ok_or("--daemon-socket requires PATH")?);
                    if !path.is_absolute() || path.file_name().is_none() {
                        return Err("daemon socket must be an absolute file path".into());
                    }
                    daemon_socket = Some(path);
                }
                _ => rest.push(arg),
            }
        }
        let mut args = rest.into_iter();
        let command = args
            .next()
            .ok_or("expected status, dump, reload, native, display or sim")?;
        let mut json = false;
        let mut output = None;
        let command = match command.as_str() {
            "status" | "dump" => {
                let mut seen = BTreeSet::new();
                while let Some(arg) = args.next() {
                    if !seen.insert(arg.clone()) {
                        return Err(format!("duplicate argument {arg}"));
                    }
                    match (command.as_str(), arg.as_str()) {
                        ("status", "--json") => json = true,
                        ("dump", "-o") => {
                            output = Some(PathBuf::from(args.next().ok_or("-o requires FILE")?))
                        }
                        _ => return Err(format!("unknown {command} argument {arg}")),
                    }
                }
                if command == "status" {
                    ControlCommand::Status
                } else {
                    ControlCommand::Session(Command::Dump)
                }
            }
            "reload" => {
                if args.as_slice().first().map(String::as_str) == Some("--daemon") {
                    args.next();
                    ControlCommand::Daemon(daemon::Command::Reload)
                } else {
                    ControlCommand::Session(Command::Reload)
                }
            }
            "native" | "display" => ControlCommand::Daemon(daemon::Command::SetMode {
                mode: if command == "native" {
                    Mode::Native
                } else {
                    Mode::Display
                },
            }),
            "sim" => {
                simulator = true;
                ControlCommand::Session(match args.next().as_deref() {
                    Some("theme") => Command::SimTheme {
                        theme: match args.next().as_deref() {
                            Some("light") => Theme::Light,
                            Some("dark") => Theme::Dark,
                            _ => return Err("sim theme requires light or dark".into()),
                        },
                    },
                    Some("fact") => {
                        let arg = args
                            .next()
                            .ok_or("sim fact requires NAME=VALUE or --unset NAME")?;
                        if arg == "--unset" {
                            Command::SimFact {
                                name: args.next().ok_or("--unset requires NAME")?.into(),
                                value: None,
                            }
                        } else {
                            let (name, value) =
                                arg.split_once('=').ok_or("sim fact requires NAME=VALUE")?;
                            Command::SimFact {
                                name: name.into(),
                                value: Some(fact_value(value)?),
                            }
                        }
                    }
                    _ => return Err("sim requires fact or theme".into()),
                })
            }
            _ => return Err(format!("unknown command {command}")),
        };
        if args.next().is_some() {
            return Err("unexpected trailing argument".into());
        }
        if simulator && (daemon_socket.is_some() || matches!(command, ControlCommand::Daemon(_))) {
            return Err("simulator commands cannot target touchbard".into());
        }
        if daemon_socket.is_some() && matches!(command, ControlCommand::Session(_)) {
            return Err("--daemon-socket requires status or a daemon command".into());
        }
        Ok(Self {
            daemon_socket,
            simulator,
            json,
            output,
            command,
        })
    }
}
fn scalar(value: serde_json::Value) -> Result<Scalar, String> {
    Ok(match value {
        serde_json::Value::Bool(v) => Scalar::Boolean(v),
        serde_json::Value::Number(v) => Scalar::Number(v.as_f64().ok_or("invalid number")?),
        serde_json::Value::String(v) => Scalar::Text(v),
        _ => return Err("record values must be scalars".into()),
    })
}
fn fact_value(text: &str) -> Result<FactValue, String> {
    use serde_json::Value;
    let value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(e) if text.starts_with(['[', '{', '"']) => return Err(e.to_string()),
        Err(_) => return Ok(FactValue::Text(text.into())),
    };
    Ok(match value {
        Value::Bool(v) => FactValue::Boolean(v),
        Value::Number(v) => FactValue::Number(v.as_f64().ok_or("invalid number")?),
        Value::String(v) => FactValue::Text(v),
        Value::Array(values) if values.iter().all(Value::is_string) => FactValue::List(
            values
                .into_iter()
                .map(|v| v.as_str().expect("string checked").into())
                .collect(),
        ),
        Value::Array(values) => FactValue::Records(
            values
                .into_iter()
                .map(|v| match v {
                    Value::Object(record) => record
                        .into_iter()
                        .map(|(k, v)| Ok::<_, String>((k, scalar(v)?)))
                        .collect(),
                    _ => Err("fact arrays must contain strings or records".into()),
                })
                .collect::<Result<_, _>>()?,
        ),
        _ => {
            return Err(
                "fact must be a scalar, string list or record list; use --unset to remove it"
                    .into(),
            );
        }
    })
}
pub fn format_status(status: &serde_json::Value) -> Result<String, String> {
    format_status_with_daemon(status, None)
}
pub(crate) fn format_status_with_daemon(
    status: &serde_json::Value,
    daemon: Option<&serde_json::Value>,
) -> Result<String, String> {
    let core: touchbar_core::CoreSnapshot =
        serde_json::from_value(status["core"].clone()).map_err(|e| e.to_string())?;
    let last_calls: Vec<(touchbar_model::InstanceId, touchbar_model::Timestamp)> =
        serde_json::from_value(status["last_calls"].clone()).map_err(|e| e.to_string())?;
    let mut text = String::new();
    writeln!(
        text,
        "Session at {} ms; {} surface(s)",
        core.now.0,
        core.surfaces.len()
    )
    .unwrap();
    for surface in &core.surfaces {
        writeln!(
            text,
            "\n{}: {}x{}, {:?}",
            surface.info.name.as_str(),
            surface.info.width_px,
            surface.info.height_px,
            surface.mode
        )
        .unwrap();
        writeln!(
            text,
            "  Profile: {:?}\n  Selection: {:?}\n  Layers: {:?}\n  Panel: {:?}",
            surface.scene.profile, surface.resolution, surface.layers, surface.presentation
        )
        .unwrap();
        if surface.info.kind == touchbar_model::SurfaceKind::Touchbar {
            if let Some(daemon) = daemon {
                crate::status::format_daemon(&mut text, daemon);
            }
            if let Some(transport) = status.get("touchbar").filter(|v| !v.is_null()) {
                crate::status::format_transport(&mut text, transport);
            }
        }
        for instance in core
            .instances
            .iter()
            .filter(|i| i.surface == surface.info.id)
        {
            let last = last_calls
                .iter()
                .find(|(id, _)| *id == instance.id)
                .map(|(_, time)| time.0);
            writeln!(
                text,
                "  Instance {:?} {:?}: {:?}, last call {:?} ms\n    State: {:?}",
                instance.id, instance.set, instance.status, last, instance.state
            )
            .unwrap();
            for subscription in &instance.subscriptions {
                writeln!(text, "    Subscription: {subscription:?}").unwrap();
            }
        }
        for request in core
            .requests
            .iter()
            .filter(|r| r.surface == surface.info.id)
        {
            writeln!(text, "  Action: {request:?}").unwrap();
        }
    }
    for kind in ["frame_ms", "wasm_ms"] {
        let metric = &status["metrics"][kind];
        writeln!(
            text,
            "{kind}: n={}, p50={} ms, p99={} ms",
            metric["samples"], metric["p50"], metric["p99"]
        )
        .unwrap();
    }
    if let Some(issues) = status["load_issues"].as_array() {
        for issue in issues {
            writeln!(
                text,
                "Load issue: {}",
                issue["message"].as_str().unwrap_or("unknown")
            )
            .unwrap();
        }
    }
    Ok(text)
}
pub fn run_control(options: ControlOptions) -> Result<String, String> {
    match &options.command {
        ControlCommand::Status if !options.simulator => return crate::status::run_status(&options),
        ControlCommand::Daemon(command) => {
            let reply = futures_lite::future::block_on(crate::status::daemon_call(
                options.daemon_socket.as_deref(),
                command.clone(),
            ))?;
            return match reply {
                daemon::Reply::Ack => Ok("OK\n".into()),
                daemon::Reply::Error(e) => Err(e),
                _ => Err("unexpected daemon control response".into()),
            };
        }
        _ => {}
    }
    let command = match options.command {
        ControlCommand::Status => Command::Status,
        ControlCommand::Session(command) => command,
        ControlCommand::Daemon(_) => unreachable!("handled above"),
    };
    let runtime = touchbar_control::runtime_dir().map_err(|e| e.to_string())?;
    let reply = futures_lite::future::block_on(touchbar_control::call(
        &touchbar_control::socket_path(&runtime, options.simulator),
        command.clone(),
    ))
    .map_err(|e| e.to_string())?;
    match (reply, &command) {
        (Reply::Error(error), _) => Err(error),
        (Reply::Ack, Command::Reload) => {
            Ok("Reload scheduled; status reports admission errors.\n".into())
        }
        (Reply::Ack, Command::SimFact { .. } | Command::SimTheme { .. }) => Ok("OK\n".into()),
        (Reply::Status(status), Command::Status) => {
            if options.json {
                serde_json::to_string_pretty(&status)
                    .map(|s| s + "\n")
                    .map_err(|e| e.to_string())
            } else {
                format_status(&status)
            }
        }
        (Reply::Dump(path), Command::Dump) => {
            if let Some(output) = options.output {
                let mut source = fs::File::open(&path).map_err(|e| e.to_string())?;
                let mut target = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&output)
                    .map_err(|e| e.to_string())?;
                target
                    .set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
                std::io::copy(&mut source, &mut target).map_err(|e| e.to_string())?;
                Ok(format!("{}\n", output.display()))
            } else {
                Ok(format!("{}\n", path.display()))
            }
        }
        _ => Err("unexpected control response".into()),
    }
}
