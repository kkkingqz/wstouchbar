use serde::{Deserialize, Serialize};
use std::{fmt, num::NonZeroU32, str::FromStr};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelError(pub String);
impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for ModelError {}

fn identifier(s: &str, plugin: bool) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || (!plugin && b == b'_')
        })
}
macro_rules! named_id {
    ($name:ident, $plugin:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);
        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl FromStr for $name {
            type Err = ModelError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                if identifier(s, $plugin) {
                    Ok(Self(s.into()))
                } else {
                    Err(ModelError(format!("invalid {}: {s}", stringify!($name))))
                }
            }
        }
        impl TryFrom<String> for $name {
            type Error = ModelError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }
        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
named_id!(PluginId, true);
named_id!(SlotId, false);
named_id!(WidgetId, false);
named_id!(PanelId, false);

macro_rules! reference {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name {
            pub plugin: PluginId,
            id: String,
        }
        impl $name {
            pub fn id(&self) -> &str {
                &self.id
            }
        }
        impl FromStr for $name {
            type Err = ModelError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let (plugin, id) = s
                    .split_once('/')
                    .ok_or_else(|| ModelError("expected plugin/id".into()))?;
                if !identifier(id, false) {
                    return Err(ModelError("invalid reference id".into()));
                }
                Ok(Self {
                    plugin: plugin.parse()?,
                    id: id.into(),
                })
            }
        }
        impl TryFrom<String> for $name {
            type Error = ModelError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }
        impl From<$name> for String {
            fn from(r: $name) -> Self {
                r.to_string()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}/{}", self.plugin, self.id)
            }
        }
    };
}
reference!(SetRef);
reference!(ProfileRef);

macro_rules! counter_id {
    ($($name:ident),+) => { $(
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    )+ };
}
counter_id!(SurfaceId, Generation, CallId, TimerId, Timestamp);
impl Timestamp {
    pub fn after(self, milliseconds: u64) -> Self {
        Self(self.0.saturating_add(milliseconds))
    }
    pub fn elapsed_since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct InstanceId {
    pub number: u64,
    pub generation: Generation,
}
macro_rules! owned_id {
    ($($name:ident),+) => { $(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name { pub instance: InstanceId, pub number: u64 }
    )+ };
}
owned_id!(RequestId, SubscriptionId);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContactId(pub NonZeroU32);
impl ContactId {
    pub fn new(number: u32) -> Option<Self> {
        NonZeroU32::new(number).map(Self)
    }
}
