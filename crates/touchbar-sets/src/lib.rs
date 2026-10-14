//! Strict plugin/set documents shared by the loader and renderer.
mod schema;
pub use schema::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use touchbar_model::PluginId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub id: PluginId,
    pub version: String,
    pub api: String,
    pub name: String,
    pub description: String,
    pub logic: Option<String>,
    #[serde(default)]
    pub requires: Vec<PluginId>,
    #[serde(default)]
    pub sets: Vec<SetEntry>,
    #[serde(default)]
    pub profiles: Vec<ProfileEntry>,
    #[serde(default)]
    pub assets: Vec<AssetEntry>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetEntry {
    pub id: String,
    pub file: String,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{file}:{line}: {path}: {message}")]
pub struct DocumentError {
    pub file: PathBuf,
    pub path: String,
    pub line: usize,
    pub message: String,
}

mod bindings;
mod document;
mod spec;
mod validation;

pub mod config;

/// Explicit manifest entry for a profile document (D85).
pub type ProfileEntry = SetEntry;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetEntry {
    pub id: String,
    pub file: String,
    pub kind: AssetKind,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssetKind {
    Png,
    SymbolicSvg,
}
