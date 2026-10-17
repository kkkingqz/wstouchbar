use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use touchbar_model::{PluginId, ProfileOverride, ProfileRef, ProfileTarget};
use touchbar_sets::{
    DocumentError,
    config::{error, parse_document},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    pub disabled: Vec<PluginId>,
    #[serde(rename = "profile-for")]
    pub profile_for: BTreeMap<String, ProfileChoice>,
    pub app: BTreeMap<String, AppConfig>,
    pub logic: LogicConfig,
    pub desktop: DesktopChoice,
    pub log: String,
}
impl Default for UserConfig {
    fn default() -> Self {
        Self {
            disabled: vec![],
            profile_for: BTreeMap::new(),
            app: BTreeMap::new(),
            logic: LogicConfig::default(),
            desktop: DesktopChoice::Auto,
            log: "info".into(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProfileChoice {
    Global(ProfileRef),
    Surface(BTreeMap<String, ProfileRef>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub categories: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogicConfig {
    #[serde(rename = "time-limit-ms")]
    pub time_limit_ms: u64,
}
impl Default for LogicConfig {
    fn default() -> Self {
        Self { time_limit_ms: 50 }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopChoice {
    #[default]
    Auto,
    Gnome,
    Null,
    Fake,
}
fn name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 128 && !n.chars().any(|c| c.is_control() || c.is_whitespace())
}
fn target(s: &str) -> Result<ProfileTarget, DocumentError> {
    match s.split_once(':') {
        Some(("app", id)) if name(id) => Ok(ProfileTarget::App(id.into())),
        Some(("category", id)) if name(id) => Ok(ProfileTarget::Category(id.into())),
        _ => Err(error(
            "profile-for",
            "target requires app:<id> or category:<id>",
        )),
    }
}
impl UserConfig {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse_document(file.as_ref(), source, Self::validate)
    }
    pub fn overrides(&self) -> Result<Vec<ProfileOverride>, DocumentError> {
        let mut out = vec![];
        for (key, choice) in &self.profile_for {
            match choice {
                ProfileChoice::Global(profile) => out.push(ProfileOverride {
                    surface: None,
                    target: target(key)?,
                    profile: profile.clone(),
                }),
                ProfileChoice::Surface(choices) => {
                    let surface: touchbar_model::SurfaceName = key
                        .parse()
                        .map_err(|_| error("profile-for", "invalid surface name"))?;
                    for (key, profile) in choices {
                        out.push(ProfileOverride {
                            surface: Some(surface.clone()),
                            target: target(key)?,
                            profile: profile.clone(),
                        });
                    }
                }
            }
        }
        Ok(out)
    }
    fn validate(&self) -> Result<(), DocumentError> {
        if self.logic.time_limit_ms < 5 {
            return Err(error(
                "logic.time-limit-ms",
                "hard limit must be at least the 5ms soft limit",
            ));
        }
        if self.disabled.len() > 256
            || self
                .disabled
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.disabled.len()
        {
            return Err(error("disabled", "requires at most 256 unique plugin ids"));
        }
        if self.profile_for.len() > 256 || self.app.len() > 1024 {
            return Err(error("config", "too many selection entries"));
        }
        self.overrides()?;
        for (id, app) in &self.app {
            if !name(id) || app.categories.len() > 128 || app.categories.iter().any(|c| !name(c)) {
                return Err(error("app", "invalid application or category"));
            }
        }
        tracing_subscriber::EnvFilter::try_new(&self.log)
            .map_err(|e| error("log", e.to_string()))?;
        Ok(())
    }
}
