use crate::{Condition, LayerSurfaces, ModelError, SetRef, WatchKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyGesture {
    Press,
    Release,
    WhileHeld,
    Hold {
        ms: u64,
    },
    Tap {
        ms: u64,
    },
    MultiTap {
        count: u32,
        within_ms: u64,
        tap_ms: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyTrigger {
    pub keys: Vec<WatchKey>,
    pub gesture: KeyGesture,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    Key(KeyTrigger),
    Sequence {
        steps: Vec<KeyTrigger>,
        within_ms: u64,
    },
}
impl Trigger {
    pub fn validate(&self) -> Result<(), ModelError> {
        fn key(t: &KeyTrigger) -> Result<(), ModelError> {
            if t.keys.is_empty() || t.keys.iter().collect::<BTreeSet<_>>().len() != t.keys.len() {
                return Err(ModelError("trigger requires unique keys".into()));
            }
            let valid = match t.gesture {
                KeyGesture::Hold { ms } | KeyGesture::Tap { ms } => ms > 0,
                KeyGesture::MultiTap {
                    count,
                    within_ms,
                    tap_ms,
                } => count >= 2 && within_ms > 0 && tap_ms > 0,
                _ => true,
            };
            if valid {
                Ok(())
            } else {
                Err(ModelError("invalid gesture timing".into()))
            }
        }
        match self {
            Self::Key(t) => key(t),
            Self::Sequence { steps, within_ms } => {
                if steps.is_empty()
                    || *within_ms == 0
                    || steps.iter().any(|s| s.gesture == KeyGesture::WhileHeld)
                {
                    return Err(ModelError(
                        "sequence requires non-held steps and positive timing".into(),
                    ));
                }
                for step in steps {
                    key(step)?;
                }
                Ok(())
            }
        }
    }
    pub fn watch_keys(&self) -> BTreeSet<WatchKey> {
        match self {
            Self::Key(t) => t.keys.iter().copied().collect(),
            Self::Sequence { steps, .. } => {
                steps.iter().flat_map(|s| s.keys.iter().copied()).collect()
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding<A> {
    pub id: String,
    pub trigger: Trigger,
    pub when: Option<Condition>,
    pub action: A,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerOperation {
    Layer(SetRef),
    ToggleLayer(SetRef),
    ClearLayers,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerAction {
    pub operation: LayerOperation,
    pub surfaces: LayerSurfaces,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingIssue {
    DuplicateId(String),
    Invalid { id: String, message: String },
    Conflict { first: String, second: String },
}

pub fn validate_bindings<A: PartialEq>(bindings: &[Binding<A>]) -> Vec<BindingIssue> {
    let mut issues = vec![];
    let mut seen = BTreeSet::new();
    for binding in bindings {
        if !seen.insert(&binding.id) {
            issues.push(BindingIssue::DuplicateId(binding.id.clone()));
        }
        if binding.id.is_empty() {
            issues.push(BindingIssue::Invalid {
                id: binding.id.clone(),
                message: "binding id is empty".into(),
            });
        }
        if let Err(error) = binding.trigger.validate() {
            issues.push(BindingIssue::Invalid {
                id: binding.id.clone(),
                message: error.to_string(),
            });
        }
        if let Some(condition) = &binding.when
            && let Err(error) = condition.validate()
        {
            issues.push(BindingIssue::Invalid {
                id: binding.id.clone(),
                message: error.to_string(),
            });
        }
    }
    fn normalized(trigger: &Trigger) -> Trigger {
        let mut trigger = trigger.clone();
        match &mut trigger {
            Trigger::Key(key) => key.keys.sort(),
            Trigger::Sequence { steps, .. } => {
                for step in steps {
                    step.keys.sort();
                }
            }
        }
        trigger
    }
    for (i, a) in bindings.iter().enumerate() {
        for b in &bindings[i + 1..] {
            if normalized(&a.trigger) == normalized(&b.trigger)
                && a.when == b.when
                && a.action != b.action
            {
                issues.push(BindingIssue::Conflict {
                    first: a.id.clone(),
                    second: b.id.clone(),
                });
            }
        }
    }
    issues
}
