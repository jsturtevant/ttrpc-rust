#[cfg(not(target_os = "windows"))]
mod linux;
#[cfg(not(target_os = "windows"))]
pub use crate::sync::sys::linux::{PipeConnection, PipeListener, ClientConnection};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use crate::sync::sys::windows::{PipeConnection, PipeListener, ClientConnection};