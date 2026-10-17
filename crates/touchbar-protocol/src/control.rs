//! Versioned JSON control on a stream socket, independent of the binary session protocol.
use crate::{Mode, invalid};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read};
pub const VERSION: u32 = touchbar_control::VERSION;
pub const MAX_FRAME: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub command: Command,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Command {
    Status,
    SetMode { mode: Mode },
    Reload,
}
impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's tagged unit variants ignore extra fields even with deny_unknown_fields.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
        enum Strict {
            Status {},
            SetMode { mode: Mode },
            Reload {},
        }
        Ok(match Strict::deserialize(deserializer)? {
            Strict::Status {} => Self::Status,
            Strict::SetMode { mode } => Self::SetMode { mode },
            Strict::Reload {} => Self::Reload,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub reply: Reply,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "data",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum Reply {
    Status(serde_json::Value),
    Ack,
    Error(String),
}
fn check_version(version: u32) -> io::Result<()> {
    if version != VERSION {
        return Err(invalid("unsupported daemon control protocol version"));
    }
    Ok(())
}
impl Request {
    pub fn new(command: Command) -> Self {
        Self {
            version: VERSION,
            command,
        }
    }
    pub fn validate(&self) -> io::Result<()> {
        check_version(self.version)
    }
}
impl Response {
    pub fn new(reply: Reply) -> Self {
        Self {
            version: VERSION,
            reply,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Reply::Error(message.into()))
    }
    pub fn validate(&self) -> io::Result<()> {
        check_version(self.version)
    }
}
pub fn encode(value: &impl Serialize) -> io::Result<Vec<u8>> {
    touchbar_control::encode_with_limit(value, MAX_FRAME)
}
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    touchbar_control::read_frame_with_limit(reader, MAX_FRAME)
}
