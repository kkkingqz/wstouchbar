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
