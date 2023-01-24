// Copyright (c) 2019 Ant Financial
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::error::Result;
use nix::sys::socket::*;
use std::io::{self, Read, Write};
use std::os::unix::io::RawFd;
use std::os::unix::prelude::AsRawFd;
use nix::sys::socket::{self, *};
use nix::unistd::*;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use crate::common;


#[derive(Clone, Copy)]
pub(crate) struct FD {
    #[cfg(target_os = "linux")]
    pub fd: RawFd,
}

pub(crate) struct LinuxListener {
    fd: RawFd,
    pub(crate) monitor_fd: (RawFd, RawFd),
}

impl AsRawFd for LinuxListener {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

impl LinuxListener {
    pub(crate) fn new_from_fd(fd: RawFd) -> Result<LinuxListener> {
        let fds = LinuxListener::new_monitor_fd()?;

        Ok(LinuxListener {
            fd,
            monitor_fd: fds,
        })
    }

    fn new_monitor_fd() ->  Result<(i32, i32)> {
        #[cfg(target_os = "linux")]
        let fds = pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
 
        
        #[cfg(not(target_os = "linux"))]
        let fds = {
            let (rfd, wfd) = pipe()?;
            set_fd_close_exec(rfd)?;
            set_fd_close_exec(wfd)?;
            (rfd, wfd)
        };


        Ok(fds)
    }

    pub(crate) fn accept(&mut self, quitFlag: &Arc<AtomicBool>) ->  std::result::Result<Option<FD>, io::Error> {
        if quitFlag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(io::ErrorKind::Other, "listener shutdown for quit flag"));
        }
        
        let mut pollers = vec![
            libc::pollfd {
                fd: self.monitor_fd.0,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let returned = unsafe {
            let pollers: &mut [libc::pollfd] = &mut pollers;
            libc::poll(
                pollers as *mut _ as *mut libc::pollfd,
                pollers.len() as _,
                -1,
            )
        };

        if returned == -1 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                return Err(err);
            }

            error!("fatal error in listener_loop:{:?}", err);
            return Err(err);
        } else if returned < 1 {
            return Ok(None)
        }

        if pollers[0].revents != 0 || pollers[pollers.len() - 1].revents == 0 {
            return Ok(None);
        }

        if quitFlag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(io::ErrorKind::Other, "listener shutdown for quit flag"));
        }

        #[cfg(target_os = "linux")]
        let fd = match accept4(self.fd, SockFlag::SOCK_CLOEXEC) {
            Ok(fd) => fd,
            Err(e) => {
                error!("failed to accept error {:?}", e);
                return Err(std::io::Error::from_raw_os_error(e as i32));
            }
        };

        // Non Linux platforms do not support accept4 with SOCK_CLOEXEC flag, so instead
        // use accept and call fcntl separately to set SOCK_CLOEXEC.
        // Because of this there is chance of the descriptor leak if fork + exec happens in between.
        #[cfg(not(target_os = "linux"))]
        let fd = match accept(listener) {
            Ok(fd) => {
                if let Err(err) = set_fd_close_exec(fd) {
                    error!("fcntl failed after accept: {:?}", err);
                    break;
                };
                fd
            }
            Err(e) => {
                error!("failed to accept error {:?}", e);
                break;
            }
        };


        Ok(Some( FD { fd } ))
    }

}


pub(crate) struct LinuxConnection {
    fd: RawFd,
}

impl LinuxConnection {
    pub(crate) fn new(fd: RawFd) -> LinuxConnection {
        LinuxConnection { fd }
    }

    pub(crate) fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match send(self.fd, &buf, MsgFlags::empty()) {
                Ok(l) => return Ok(l),
                Err(e) if retryable(e) => {
                    // Should retry
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }

    pub(crate) fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match  recv(self.fd, buf, MsgFlags::empty()) {
                Ok(l) => return Ok(l),
                Err(e) if retryable(e) => {
                    // Should retry
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
        
       
    }
}


fn retryable(e: nix::Error) -> bool {
    use ::nix::Error;
    e == Error::EINTR || e == Error::EAGAIN
}
