use std::process::{Command, ExitCode};

#[derive(Clone, Copy)]
enum Environment {
    Gpu,
    Dbus,
    Pipewire,
}
impl Environment {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "gpu" => Some(Self::Gpu),
            "dbus" => Some(Self::Dbus),
            "pipewire" => Some(Self::Pipewire),
            _ => None,
        }
    }
    fn filter(self) -> &'static str {
        match self {
            Self::Gpu => "env_gpu_",
            Self::Dbus => "env_dbus_",
            Self::Pipewire => "env_pipewire_",
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
    if args.first().map(String::as_str) == Some("build-plugins") {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root");
        let output = match &args[1..] {
            [] => root.join("target/plugins"),
            [flag, path] if flag == "--out" => path.into(),
            _ => {
                eprintln!("Usage: cargo xtask build-plugins [--out DIR]");
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
            "Usage: cargo xtask test-env gpu|dbus|pipewire; cargo xtask build-plugins [--out DIR]"
        );
        return ExitCode::SUCCESS;
    }
    let [command, name] = args.as_slice() else {
        eprintln!(
            "Usage: cargo xtask test-env gpu|dbus|pipewire; cargo xtask build-plugins [--out DIR]"
        );
        return ExitCode::from(2);
    };
    let Some(environment) = Environment::parse(name).filter(|_| command == "test-env") else {
        eprintln!(
            "Usage: cargo xtask test-env gpu|dbus|pipewire; cargo xtask build-plugins [--out DIR]"
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
