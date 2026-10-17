//! Directional v2 privilege-boundary protocol. Parsing never performs device IO.
pub mod control;
mod swapchain;
mod transport;
mod wire;
use std::io;
pub use swapchain::*;
pub use transport::*;
pub use wire::*;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
