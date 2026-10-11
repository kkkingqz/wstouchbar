use crate::ModelError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FactName(pub String);
impl From<&str> for FactName {
    fn from(s: &str) -> Self {
        Self(s.into())
    }
}
impl From<String> for FactName {
    fn from(s: String) -> Self {
        Self(s)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Scalar {
    Boolean(bool),
    Number(#[serde(with = "crate::wire_float::f64")] f64),
    Text(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScalarKind {
    Boolean,
    Number,
    Text,
}
impl Scalar {
    pub fn kind(&self) -> ScalarKind {
        match self {
            Self::Boolean(_) => ScalarKind::Boolean,
            Self::Number(_) => ScalarKind::Number,
            Self::Text(_) => ScalarKind::Text,
        }
    }
    pub fn validate(&self) -> Result<(), ModelError> {
        if matches!(self, Self::Number(n) if !n.is_finite()) {
            Err(ModelError("number must be finite".into()))
        } else {
            Ok(())
        }
    }
}
pub type RecordValue = BTreeMap<String, Scalar>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FactValue {
    Boolean(bool),
    Number(#[serde(with = "crate::wire_float::f64")] f64),
    Text(String),
    List(Vec<String>),
    Records(Vec<RecordValue>),
}
impl FactValue {
    pub fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::Number(n) if !n.is_finite() => {
                Err(ModelError("fact number must be finite".into()))
            }
            Self::Records(records) => {
                for r in records {
                    for scalar in r.values() {
                        scalar.validate()?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
pub type Facts = BTreeMap<FactName, FactValue>;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    Eq {
        name: FactName,
        value: FactValue,
    },
    In {
        name: FactName,
        values: Vec<FactValue>,
    },
    Exists(FactName),
    Not(Box<Condition>),
    All(Vec<Condition>),
    Any(Vec<Condition>),
}
impl Condition {
    pub fn evaluate(&self, facts: &Facts) -> bool {
        match self {
            Self::Eq { name, value } => facts.get(name) == Some(value),
            Self::In { name, values } => {
                facts.get(name).is_some_and(|value| values.contains(value))
            }
            Self::Exists(name) => facts.contains_key(name),
            Self::Not(condition) => !condition.evaluate(facts),
            Self::All(conditions) => conditions.iter().all(|c| c.evaluate(facts)),
            Self::Any(conditions) => conditions.iter().any(|c| c.evaluate(facts)),
        }
    }
    pub fn validate(&self) -> Result<(), ModelError> {
        fn depth(c: &Condition, n: u8) -> Result<(), ModelError> {
            if n > 8 {
                return Err(ModelError("condition depth exceeds 8".into()));
            }
            match c {
                Condition::Eq { value, .. } => value.validate(),
                Condition::In { values, .. } => {
                    for value in values {
                        value.validate()?;
                    }
                    Ok(())
                }
                Condition::Not(c) => depth(c, n + 1),
                Condition::All(cs) | Condition::Any(cs) => {
                    for c in cs {
                        depth(c, n + 1)?;
                    }
                    Ok(())
                }
                _ => Ok(()),
            }
        }
        depth(self, 1)
    }
}
