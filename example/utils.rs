#![allow(dead_code)]
use std::fs;
use std::io::Result;
use std::path::Path;

#[cfg(target_os = "linux")]
pub const SOCK_ADDR: &str = r"unix:///tmp/ttrpc-test";

#[cfg(target_os = "windows")]
pub const SOCK_ADDR: &str = r"\\.\pipe\mio-named-pipe-test";

#[cfg(target_os = "linux")]
pub fn remove_if_sock_exist(sock_addr: &str) -> Result<()> {
    let path = sock_addr
        .strip_prefix("unix://")
        .expect("socket address is not expected");

    if Path::new(path).exists() {
        fs::remove_file(path)?;
    }

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn remove_if_sock_exist(sock_addr: &str) -> Result<()> {
    //todo close file handle

    Ok(())
}

