//! Strict layered daemon configuration. A failed reload never changes the accepted config.
use serde::Deserialize;
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};
use touchbar_bindings::{
    Binding,
    config::{BindingEntry, BindingFile, RecoveryAction, merge_bindings},
};
use touchbar_sets::{
    DocumentError,
    config::{error, parse_document},
};
pub const DEFAULT_CONFIG: &str = include_str!("../../../packaging/touchbard.toml");
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct ConfigFile {
    #[serde(default, rename = "binding")]
    bindings: Vec<BindingEntry<RecoveryAction>>,
    session_grace_ms: Option<u64>,
    dim_timeout: Option<u64>,
    idle_timeout: Option<u64>,
    backlight: Option<BacklightFile>,
    log: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct BacklightFile {
    active: Option<u8>,
    dimmed: Option<u8>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BacklightLevels {
    pub active: u8,
    pub dimmed: u8,
}
impl Default for BacklightLevels {
    fn default() -> Self {
        Self {
            active: 2,
            dimmed: 1,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub bindings: Vec<Binding<RecoveryAction>>,
    pub session_grace_ms: u64,
    /// Seconds until dimming; None is resolved from driver parameters by the IO shell.
    pub dim_timeout: Option<u64>,
    /// Additional seconds after dimming until off; None uses driver parameters.
    pub idle_timeout: Option<u64>,
    pub backlight: BacklightLevels,
    pub log: String,
}
impl Default for Config {
    fn default() -> Self {
        let file = ConfigFile::from_toml("built-in touchbard.toml", DEFAULT_CONFIG)
            .expect("valid shipped defaults");
        Self::merge(&file, None).expect("valid shipped defaults")
    }
}
impl ConfigFile {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse_document(file.as_ref(), source, Self::validate)
    }
    fn binding_file(&self) -> BindingFile<RecoveryAction> {
        BindingFile {
            bindings: self.bindings.clone(),
        }
    }
    fn validate(&self) -> Result<(), DocumentError> {
        self.binding_file().to_bindings()?;
        for (name, seconds) in [
            ("dim-timeout", self.dim_timeout),
            ("idle-timeout", self.idle_timeout),
        ] {
            if seconds.is_some_and(|s| s.checked_mul(1000).is_none()) {
                return Err(error(name, "timeout does not fit millisecond timestamp"));
            }
        }
        if let Some(b) = &self.backlight {
            if b.active.is_some_and(|v| !(1..=2).contains(&v)) {
                return Err(error("backlight.active", "active level must be 1..2"));
            }
            if b.dimmed.is_some_and(|v| v > 2) || b.active.zip(b.dimmed).is_some_and(|(a, d)| d > a)
            {
                return Err(error(
                    "backlight.dimmed",
                    "dimmed level must be 0..active (at most 2)",
                ));
            }
        }
        if let Some(log) = &self.log {
            tracing_subscriber::EnvFilter::try_new(log).map_err(|e| error("log", e.to_string()))?;
        }
        Ok(())
    }
}
impl Config {
    pub fn merge(base: &ConfigFile, admin: Option<&ConfigFile>) -> Result<Self, DocumentError> {
        base.validate()?;
        if let Some(admin) = admin {
            admin.validate()?;
        }
        let bindings = merge_bindings(
            &[base.binding_file()],
            admin.map(ConfigFile::binding_file).as_ref(),
        )?;
        let choose = |a: Option<u64>, b: Option<u64>| b.or(a);
        let mut result = Self {
            bindings,
            session_grace_ms: choose(
                base.session_grace_ms,
                admin.and_then(|a| a.session_grace_ms),
            )
            .unwrap_or(2000),
            dim_timeout: choose(base.dim_timeout, admin.and_then(|a| a.dim_timeout)),
            idle_timeout: choose(base.idle_timeout, admin.and_then(|a| a.idle_timeout)),
            backlight: BacklightLevels::default(),
            log: admin
                .and_then(|a| a.log.clone())
                .or_else(|| base.log.clone())
                .unwrap_or_else(|| "info".into()),
        };
        for file in [Some(base), admin].into_iter().flatten() {
            if let Some(b) = &file.backlight {
                if let Some(v) = b.active {
                    result.backlight.active = v;
                }
                if let Some(v) = b.dimmed {
                    result.backlight.dimmed = v;
                }
            }
        }
        if result.backlight.dimmed > result.backlight.active {
            return Err(error(
                "backlight.dimmed",
                "dimmed exceeds active level after merge",
            ));
        }
        if let Some((dim, idle)) = result.dim_timeout.zip(result.idle_timeout)
            && dim
                .checked_add(idle)
                .and_then(|s| s.checked_mul(1000))
                .is_none()
        {
            return Err(error(
                "idle-timeout",
                "combined timeouts overflow millisecond timestamp",
            ));
        }
        Ok(result)
    }
}
#[derive(Clone, Debug, Default)]
pub struct ConfigStore {
    current: Config,
}
impl ConfigStore {
    pub fn current(&self) -> &Config {
        &self.current
    }
    /// The caller reports Err as a warning; the complete previous config remains accepted.
    pub fn reload(
        &mut self,
        distribution: &Path,
        administrator: &Path,
    ) -> Result<(), DocumentError> {
        let base = read_optional(distribution)?.unwrap_or(ConfigFile::from_toml(
            "built-in touchbard.toml",
            DEFAULT_CONFIG,
        )?);
        let admin = read_optional(administrator)?;
        let proposed = Config::merge(&base, admin.as_ref()).map_err(|mut e| {
            e.file = if admin.is_some() {
                administrator.into()
            } else {
                distribution.into()
            };
            e
        })?;
        self.current = proposed;
        Ok(())
    }
}
fn read_optional(path: &Path) -> Result<Option<ConfigFile>, DocumentError> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path, e)),
    };
    let mut source = String::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut source)
        .map_err(|e| io_error(path, e))?;
    ConfigFile::from_toml(path, &source).map(Some)
}
fn io_error(path: &Path, e: io::Error) -> DocumentError {
    let mut d = error("$", e.to_string());
    d.file = path.into();
    d
}
