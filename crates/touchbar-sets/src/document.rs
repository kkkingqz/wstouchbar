use crate::{DocumentError, PluginManifest, SetFile};
use serde::{Deserialize, de::DeserializeOwned};
use std::{ops::Range, path::Path};
use toml::de::{DeTable, DeValue};

pub const MAX_CONFIG_BYTES: usize = 256 * 1024;
#[derive(Default)]
struct SourceIndex(Vec<(String, Range<usize>)>);
impl SourceIndex {
    fn table(
        &mut self,
        table: &DeTable<'_>,
        path: &str,
        depth: usize,
    ) -> Result<(), DocumentError> {
        if depth > 136 {
            return Err(error(path, "TOML structure exceeds depth budget"));
        }
        for (key, value) in table {
            let child = join(path, key.get_ref());
            self.0.push((child.clone(), key.span()));
            self.0.push((child.clone(), value.span()));
            self.value(value.get_ref(), &child, depth + 1)?;
        }
        Ok(())
    }
    fn value(
        &mut self,
        value: &DeValue<'_>,
        path: &str,
        depth: usize,
    ) -> Result<(), DocumentError> {
        if depth > 136 {
            return Err(error(path, "TOML structure exceeds depth budget"));
        }
        match value {
            DeValue::Table(t) => self.table(t, path, depth + 1)?,
            DeValue::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    let child = format!("{path}[{i}]");
                    self.0.push((child.clone(), v.span()));
                    self.value(v.get_ref(), &child, depth + 1)?;
                }
            }
            DeValue::Float(n) if n.as_str().parse::<f64>().is_ok_and(|n| !n.is_finite()) => {
                return Err(error(path, "value must be finite"));
            }
            DeValue::Datetime(_) => return Err(error(path, "dates must be quoted text")),
            _ => {}
        }
        Ok(())
    }
    fn locate(&self, path: &str) -> usize {
        let mut path = path;
        loop {
            if let Some((_, s)) = self.0.iter().find(|(p, _)| p == path) {
                return s.start;
            }
            let Some(i) = path.rfind(['.', '[']) else {
                return 0;
            };
            path = &path[..i];
        }
    }
}
fn join(parent: &str, child: &str) -> String {
    if parent == "$" {
        child.into()
    } else {
        format!("{parent}.{child}")
    }
}
pub(crate) fn error(path: &str, message: impl Into<String>) -> DocumentError {
    DocumentError {
        file: "set".into(),
        path: path.into(),
        line: 1,
        message: message.into(),
    }
}
fn decorate(
    mut error: DocumentError,
    file: &Path,
    source: &str,
    index: &SourceIndex,
) -> DocumentError {
    error.file = file.into();
    error.line = line(source, index.locate(&error.path));
    error
}
fn line(source: &str, offset: usize) -> usize {
    1 + source.as_bytes()[..offset.min(source.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
}
pub(crate) fn parse<T: DeserializeOwned>(
    file: &Path,
    source: &str,
    validate: impl FnOnce(&T) -> Result<(), DocumentError>,
) -> Result<T, DocumentError> {
    if source.len() > MAX_CONFIG_BYTES {
        let mut e = error("$", "file exceeds 256 KiB");
        e.file = file.into();
        return Err(e);
    }
    let table = DeTable::parse(source).map_err(|e| DocumentError {
        file: file.into(),
        path: "$".into(),
        line: line(source, e.span().map_or(0, |s| s.start)),
        message: e.message().into(),
    })?;
    let mut index = SourceIndex::default();
    index
        .table(table.get_ref(), "$", 0)
        .map_err(|e| decorate(e, file, source, &index))?;
    // JSON's compact deserializer avoids the large recursive TOML visitor stack.
    // Non-finite numbers and dates were rejected before this conversion.
    let raw = toml::Value::deserialize(toml::de::Deserializer::from(table))
        .map_err(|e| decorate(error("$", e.message()), file, source, &index))?;
    let json = serde_json::to_value(raw)
        .map_err(|e| decorate(error("$", e.to_string()), file, source, &index))?;
    let value = serde_path_to_error::deserialize::<_, T>(json).map_err(|e| {
        let mut path = e.path().to_string();
        if path.is_empty() || path == "." {
            path = "$".into();
        }
        let message = e.inner().to_string();
        for prefix in ["missing field `"] {
            if let Some(field) = message
                .strip_prefix(prefix)
                .and_then(|s| s.split('`').next())
            {
                path = join(&path, field);
                break;
            }
        }
        decorate(error(&path, message), file, source, &index)
    })?;
    validate(&value).map_err(|e| decorate(e, file, source, &index))?;
    Ok(value)
}
impl SetFile {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse(file.as_ref(), source, Self::validate)
    }
}
impl PluginManifest {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse(file.as_ref(), source, Self::validate)
    }
}

impl SetFile {
    pub fn from_toml_with_description(
        file: impl AsRef<Path>,
        source: &str,
        description: &touchbar_model::LogicDescription,
    ) -> Result<Self, DocumentError> {
        parse(file.as_ref(), source, |set: &Self| {
            set.validate()?;
            set.validate_bindings(description)
        })
    }
}
