//! Shared binding file grammar, parameterized by the action document type.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeSet, path::Path};
use touchbar_model::{
    Binding, BindingIssue, KeyGesture, KeyTrigger, LayerAction, LayerOperation, LayerSurfaces,
    SetRef, SurfaceName, Trigger, WatchKey, validate_bindings,
};
use touchbar_sets::{
    DocumentError,
    config::{When, error, parse_document},
};

pub trait ActionDocument: Clone + DeserializeOwned {
    type Action: Clone + PartialEq;
    fn to_action(&self) -> Result<Self::Action, String>;
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "A: Deserialize<'de>"))]
pub struct BindingFile<A> {
    #[serde(default, rename = "binding")]
    pub bindings: Vec<BindingEntry<A>>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingEntry<A> {
    pub id: String,
    pub trigger: Option<TriggerFile>,
    pub action: Option<A>,
    pub when: Option<When>,
    #[serde(default)]
    pub disabled: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerFile {
    pub key: Option<WatchKey>,
    pub keys: Option<Vec<WatchKey>>,
    pub gesture: Option<GestureFile>,
    pub ms: Option<u64>,
    pub count: Option<u32>,
    #[serde(rename = "within-ms")]
    pub within_ms: Option<u64>,
    pub sequence: Option<Vec<TriggerFile>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GestureFile {
    Press,
    Release,
    WhileHeld,
    Hold,
    Tap,
    MultiTap,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerActionFile {
    pub layer: Option<SetRef>,
    #[serde(rename = "toggle-layer")]
    pub toggle_layer: Option<SetRef>,
    #[serde(rename = "clear-layers")]
    pub clear_layers: Option<bool>,
    pub surfaces: Option<SurfacesFile>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SurfacesFile {
    All(String),
    Names(Vec<SurfaceName>),
}
impl ActionDocument for LayerActionFile {
    type Action = LayerAction;
    fn to_action(&self) -> Result<LayerAction, String> {
        if usize::from(self.layer.is_some())
            + usize::from(self.toggle_layer.is_some())
            + usize::from(self.clear_layers.is_some())
            != 1
            || self.clear_layers == Some(false)
        {
            return Err(
                "action requires exactly one of layer, toggle-layer or clear-layers=true".into(),
            );
        }
        let operation = if let Some(layer) = &self.layer {
            LayerOperation::Layer(layer.clone())
        } else if let Some(layer) = &self.toggle_layer {
            LayerOperation::ToggleLayer(layer.clone())
        } else {
            LayerOperation::ClearLayers
        };
        let surfaces = match &self.surfaces {
            None => LayerSurfaces::Source,
            Some(SurfacesFile::All(s)) if s == "all" => LayerSurfaces::All,
            Some(SurfacesFile::All(_)) => return Err("surfaces string must be all".into()),
            Some(SurfacesFile::Names(names)) => {
                if names.is_empty() || names.iter().collect::<BTreeSet<_>>().len() != names.len() {
                    return Err("surfaces requires nonempty unique names".into());
                }
                LayerSurfaces::Names(names.clone())
            }
        };
        Ok(LayerAction {
            operation,
            surfaces,
        })
    }
}
impl TriggerFile {
    fn to_trigger(&self, path: &str) -> Result<Trigger, DocumentError> {
        let trigger = if let Some(steps) = &self.sequence {
            if self.key.is_some()
                || self.keys.is_some()
                || self.gesture.is_some()
                || self.ms.is_some()
                || self.count.is_some()
            {
                return Err(error(path, "sequence cannot contain key-gesture fields"));
            }
            let within_ms = self.within_ms.ok_or_else(|| {
                error(&format!("{path}.within-ms"), "sequence requires within-ms")
            })?;
            let steps = steps
                .iter()
                .enumerate()
                .map(|(i, s)| s.to_key(&format!("{path}.sequence[{i}]")))
                .collect::<Result<Vec<_>, _>>()?;
            Trigger::Sequence { steps, within_ms }
        } else {
            Trigger::Key(self.to_key(path)?)
        };
        trigger.validate().map_err(|e| error(path, e.0))?;
        Ok(trigger)
    }
    fn to_key(&self, path: &str) -> Result<KeyTrigger, DocumentError> {
        if self.sequence.is_some() {
            return Err(error(path, "nested sequence is forbidden"));
        }
        let keys = match (&self.key, &self.keys) {
            (Some(key), None) => vec![*key],
            (None, Some(keys)) => keys.clone(),
            _ => return Err(error(path, "trigger requires exactly one of key or keys")),
        };
        let kind = self
            .gesture
            .ok_or_else(|| error(&format!("{path}.gesture"), "key trigger requires gesture"))?;
        if self.count.is_some() && kind != GestureFile::MultiTap
            || self.within_ms.is_some() && kind != GestureFile::MultiTap
            || self.ms.is_some()
                && !matches!(
                    kind,
                    GestureFile::Hold | GestureFile::Tap | GestureFile::MultiTap
                )
        {
            return Err(error(path, "gesture has unused timing/count parameters"));
        }
        let gesture = match kind {
            GestureFile::Press => KeyGesture::Press,
            GestureFile::Release => KeyGesture::Release,
            GestureFile::WhileHeld => KeyGesture::WhileHeld,
            GestureFile::Hold => KeyGesture::Hold {
                ms: self
                    .ms
                    .ok_or_else(|| error(&format!("{path}.ms"), "hold requires ms"))?,
            },
            GestureFile::Tap => KeyGesture::Tap {
                ms: self.ms.unwrap_or(300),
            },
            GestureFile::MultiTap => KeyGesture::MultiTap {
                count: self
                    .count
                    .ok_or_else(|| error(&format!("{path}.count"), "multi-tap requires count"))?,
                within_ms: self.within_ms.ok_or_else(|| {
                    error(&format!("{path}.within-ms"), "multi-tap requires within-ms")
                })?,
                tap_ms: self.ms.unwrap_or(300),
            },
        };
        let trigger = KeyTrigger { keys, gesture };
        Trigger::Key(trigger.clone())
            .validate()
            .map_err(|e| error(path, e.0))?;
        Ok(trigger)
    }
}
impl<A: ActionDocument> BindingFile<A> {
    pub fn from_toml(file: impl AsRef<Path>, source: &str) -> Result<Self, DocumentError> {
        parse_document(file.as_ref(), source, |f: &Self| {
            f.to_bindings().map(|_| ())
        })
    }
    pub fn to_bindings(&self) -> Result<Vec<Binding<A::Action>>, DocumentError> {
        let mut ids = BTreeSet::new();
        let mut bindings = Vec::new();
        let mut positions = Vec::new();
        for (i, entry) in self.bindings.iter().enumerate() {
            let p = format!("binding[{i}]");
            if entry.id.is_empty()
                || entry.id.len() > 128
                || entry
                    .id
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(error(&format!("{p}.id"), "invalid binding id"));
            }
            if !ids.insert(&entry.id) {
                return Err(error(&format!("{p}.id"), "duplicate binding id"));
            }
            if entry.disabled {
                if entry.trigger.is_some() || entry.action.is_some() || entry.when.is_some() {
                    return Err(error(
                        &p,
                        "disabled override contains only id and disabled=true",
                    ));
                }
                continue;
            }
            let trigger = entry
                .trigger
                .as_ref()
                .ok_or_else(|| error(&format!("{p}.trigger"), "binding requires trigger"))?
                .to_trigger(&format!("{p}.trigger"))?;
            let action = entry
                .action
                .as_ref()
                .ok_or_else(|| error(&format!("{p}.action"), "binding requires action"))?
                .to_action()
                .map_err(|e| error(&format!("{p}.action"), e))?;
            let when = entry
                .when
                .as_ref()
                .map(|w| w.to_condition(&format!("{p}.when")))
                .transpose()?;
            positions.push(i);
            bindings.push(Binding {
                id: entry.id.clone(),
                trigger,
                action,
                when,
            });
        }
        check_conflicts(&bindings, Some(&positions))?;
        Ok(bindings)
    }
}
fn check_conflicts<A: PartialEq>(
    bindings: &[Binding<A>],
    positions: Option<&[usize]>,
) -> Result<(), DocumentError> {
    if let Some(issue) = validate_bindings(bindings).first() {
        let id = match issue {
            BindingIssue::DuplicateId(id) | BindingIssue::Invalid { id, .. } => id,
            BindingIssue::Conflict { second, .. } => second,
        };
        let i = bindings.iter().position(|b| &b.id == id).unwrap_or(0);
        let i = positions.map_or(i, |positions| positions[i]);
        return Err(error(&format!("binding[{i}]"), format!("{issue:?}")));
    }
    Ok(())
}
pub fn merge_bindings<A: ActionDocument>(
    plugins: &[BindingFile<A>],
    user: Option<&BindingFile<A>>,
) -> Result<Vec<Binding<A::Action>>, DocumentError> {
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    for plugin in plugins {
        for binding in plugin.to_bindings()? {
            if !ids.insert(binding.id.clone()) {
                return Err(error(
                    "binding",
                    format!("duplicate plugin binding id {}", binding.id),
                ));
            }
            result.push(binding);
        }
    }
    if let Some(user) = user {
        let overrides = user.to_bindings()?;
        for entry in &user.bindings {
            let position = result.iter().position(|b| b.id == entry.id);
            if entry.disabled {
                if let Some(i) = position {
                    result.remove(i);
                }
                continue;
            }
            let binding = overrides
                .iter()
                .find(|b| b.id == entry.id)
                .expect("validated enabled override")
                .clone();
            if let Some(i) = position {
                result[i] = binding;
            } else {
                result.push(binding);
            }
        }
    }
    check_conflicts(&result, None)?;
    Ok(result)
}
