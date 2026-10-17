//! Bounded, versioned, same-UID session control. One request per connection.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use touchbar_model::{FactName, FactValue};
pub mod server;
pub use server::{Incoming, Server, check_peer};
pub const VERSION: u32 = 1;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub version: u32,
    pub command: Command,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Command {
    Status,
    Dump,
    Reload,
    SimFact {
        name: FactName,
        value: Option<FactValue>,
    },
    SimTheme {
        theme: Theme,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}
impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub reply: Reply,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "kebab-case")]
pub enum Reply {
    Status(serde_json::Value),
    Dump(PathBuf),
    Ack,
    Error(String),
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
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
fn check_version(version: u32) -> io::Result<()> {
    if version != VERSION {
        return Err(invalid(format!(
            "unsupported control protocol version {version}; expected {VERSION}"
        )));
    }
    Ok(())
}
fn frame_length(prefix: [u8; 4]) -> io::Result<usize> {
    let size = usize::try_from(u32::from_le_bytes(prefix)).map_err(|e| invalid(e.to_string()))?;
    if size == 0 || size > MAX_FRAME {
        return Err(invalid("control frame length must be 1..16 MiB"));
    }
    Ok(size)
}
/// JSON serialization stops at the budget rather than allocating an oversized frame.
pub fn encode(value: &impl Serialize) -> io::Result<Vec<u8>> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > MAX_FRAME + 4 {
                return Err(invalid("control frame exceeds 16 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(vec![0; 4]);
    serde_json::to_writer(&mut output, value).map_err(|e| invalid(e.to_string()))?;
    let length = u32::try_from(output.0.len() - 4).map_err(|e| invalid(e.to_string()))?;
    output.0[..4].copy_from_slice(&length.to_le_bytes());
    Ok(output.0)
}
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> io::Result<T> {
    let mut prefix = [0; 4];
    reader.read_exact(&mut prefix)?;
    let mut bytes = vec![0; frame_length(prefix)?];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))
}
async fn read_async<T: DeserializeOwned>(
    stream: &mut async_io::Async<std::os::unix::net::UnixStream>,
) -> io::Result<T> {
    use futures_lite::io::AsyncReadExt;
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await?;
    let mut bytes = vec![0; frame_length(prefix)?];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(e.to_string()))
}
pub fn socket_path(runtime: &Path, simulator: bool) -> PathBuf {
    let dir = runtime.join("touchbar");
    if simulator {
        dir.join("simulator/session-control.sock")
    } else {
        dir.join("session-control.sock")
    }
}
pub fn state_home(home: &Path, simulator: bool) -> PathBuf {
    if simulator {
        home.join("simulator")
    } else {
        home.into()
    }
}
pub fn runtime_dir() -> io::Result<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| invalid("XDG_RUNTIME_DIR must be an absolute directory"))
}
pub async fn call(path: &Path, command: Command) -> io::Result<Reply> {
    use futures_lite::io::AsyncWriteExt;
    futures_lite::future::or(
        async {
            let mut stream =
                async_io::Async::<std::os::unix::net::UnixStream>::connect(path).await?;
            check_peer(stream.get_ref(), rustix::process::getuid().as_raw())?;
            stream.write_all(&encode(&Request::new(command))?).await?;
            let response: Response = read_async(&mut stream).await?;
            check_version(response.version)?;
            Ok(response.reply)
        },
        async {
            async_io::Timer::after(std::time::Duration::from_secs(10)).await;
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "session control timed out",
            ))
        },
    )
    .await
}
