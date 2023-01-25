#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use crate::sync::sys::linux::{PipeConnection, PipeListener};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use crate::sync::sys::linux::net::{FD, LinuxListener, LinuxConnection};