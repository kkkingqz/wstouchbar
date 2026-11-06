use std::process::{Command, ExitCode};

#[derive(Clone, Copy)]
enum Environment {
    Gpu,
    Dbus,
    Pipewire,
    Hardware,
    Packaging,
}
impl Environment {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "gpu" => Some(Self::Gpu),
            "dbus" => Some(Self::Dbus),
            "pipewire" => Some(Self::Pipewire),
            "hardware" => Some(Self::Hardware),
            "packaging" => Some(Self::Packaging),
            _ => None,
        }
    }
    fn filter(self) -> &'static str {
        match self {
            Self::Gpu => "env_gpu_",
            Self::Dbus => "env_dbus_",
            Self::Pipewire => "env_pipewire_",
            Self::Hardware => "env_hardware_",
            Self::Packaging => "env_packaging_",
        }
    }
    fn check(self) -> Result<(), String> {
        match self {
            Self::Gpu => {
                let out = Command::new("ldconfig")
                    .arg("-p")
                    .output()
                    .map_err(|e| format!("GPU library inspection requires ldconfig: {e}"))?;
                if !out.status.success() {
                    return Err("ldconfig failed while checking GPU libraries".into());
                }
                let libraries = String::from_utf8_lossy(&out.stdout);
                for name in ["libEGL.so.1", "libGLESv2.so.2"] {
                    if !libraries
                        .lines()
                        .any(|line| line.split_whitespace().next() == Some(name))
                    {
                        return Err(format!(
                            "GPU environment requires {name}; install Mesa EGL/GLES runtime libraries"
                        ));
                    }
                }
                Ok(())
            }
            Self::Hardware => xtask::check_hardware(
                std::path::Path::new("/sys/bus/usb/devices"),
                rustix::process::geteuid().as_raw(),
            ),
            Self::Packaging => {
                check_program("systemd-analyze")?;
                check_program("udevadm")
            }
            Self::Dbus => check_program("dbus-daemon"),
            Self::Pipewire => {
                check_program("pipewire")?;
                check_program("wireplumber")
            }
        }
    }
}
fn check_program(program: &str) -> Result<(), String> {
    let output = Command::new(program)
        .arg("--version")
        .output()
        .map_err(|e| format!("test environment requires {program}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program} environment probe failed: {}",
            output.status
        ))
    }
}
fn run(environment: Environment) -> Result<u8, String> {
    environment.check()?;
    // Environment tests own private services/contexts. Preflight never contacts
    // the user's D-Bus or PipeWire and never launches a host service.
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args([
        "test",
        "--workspace",
        "--locked",
        environment.filter(),
        "--",
        "--ignored",
        "--nocapture",
    ]);
    if matches!(environment, Environment::Gpu) {
        cmd.env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("EGL_PLATFORM", "surfaceless");
    }
    let status = cmd
        .status()
        .map_err(|e| format!("could not run environment tests: {e}"))?;
    Ok(status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .unwrap_or(1))
}
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if matches!(
        args.first().map(String::as_str),
        Some("install" | "uninstall")
    ) {
        return match install_command(&args) {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("build-plugins") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let output = match &args[1..] {
            [] => root.join("target/plugins"),
            [flag, path] if flag == "--out" => path.into(),
            _ => {
                eprintln!(
                    "Usage: cargo xtask build-plugins [--out DIR]; cargo xtask install --destdir DIR [--prefix /usr] [--profile release|dev]; cargo xtask uninstall --manifest FILE"
                );
                return ExitCode::from(2);
            }
        };
        return match xtask::build_plugins(root, &output) {
            Ok(()) => {
                println!("Built plugins: {}", output.display());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("xtask: {error}");
                ExitCode::FAILURE
            }
        };
    }
    if args.as_slice() == ["--help"] || args.as_slice() == ["-h"] {
        println!(
            "Usage: cargo xtask test-env gpu|dbus|pipewire|hardware|packaging; cargo xtask build-plugins [--out DIR]; cargo xtask install --destdir DIR [--prefix /usr] [--profile release|dev]; cargo xtask uninstall --manifest FILE"
        );
        return ExitCode::SUCCESS;
    }
    let [command, name] = args.as_slice() else {
        eprintln!(
            "Usage: cargo xtask test-env gpu|dbus|pipewire|hardware|packaging; cargo xtask build-plugins [--out DIR]; cargo xtask install --destdir DIR [--prefix /usr] [--profile release|dev]; cargo xtask uninstall --manifest FILE"
        );
        return ExitCode::from(2);
    };
    let Some(environment) = Environment::parse(name).filter(|_| command == "test-env") else {
        eprintln!(
            "Usage: cargo xtask test-env gpu|dbus|pipewire|hardware|packaging; cargo xtask build-plugins [--out DIR]; cargo xtask install --destdir DIR [--prefix /usr] [--profile release|dev]; cargo xtask uninstall --manifest FILE"
        );
        return ExitCode::from(2);
    };
    match run(environment) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn install_command(args: &[String]) -> Result<String, String> {
    use std::path::{Path, PathBuf};
    let mut flags = std::collections::BTreeMap::new();
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let (key, value) = if let Some((k, v)) = arg.split_once('=') {
            (k, v.to_owned())
        } else {
            let value = rest
                .next()
                .filter(|v| !v.starts_with("--"))
                .ok_or_else(|| format!("{arg} requires a value"))?;
            (arg.as_str(), value.clone())
        };
        if !matches!(key, "--prefix" | "--destdir" | "--profile" | "--manifest")
            || flags.insert(key, value).is_some()
        {
            return Err(format!("unknown or duplicate argument {key}"));
        }
    }
    if args[0] == "uninstall" {
        let file = flags
            .remove("--manifest")
            .ok_or("uninstall requires --manifest FILE")?;
        if !flags.is_empty() {
            return Err("uninstall only accepts --manifest".into());
        }
        xtask::install::uninstall(Path::new(&file))?;
        return Ok(format!("Uninstalled package files from {}", file));
    }
    let out = PathBuf::from(
        flags
            .remove("--destdir")
            .ok_or("install requires --destdir DIR; it builds a staging tree only")?,
    );
    let prefix = PathBuf::from(flags.remove("--prefix").unwrap_or_else(|| "/usr".into()));
    let profile = flags
        .remove("--profile")
        .unwrap_or_else(|| "release".into());
    if !flags.is_empty() {
        return Err("install does not accept --manifest".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let manifest = xtask::install::build(root, &out, &prefix, &profile)?;
    Ok(format!("Install manifest: {}", manifest.display()))
}
