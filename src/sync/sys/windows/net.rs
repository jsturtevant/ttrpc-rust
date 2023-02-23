/*
	Copyright The containerd Authors.

	Licensed under the Apache License, Version 2.0 (the "License");
	you may not use this file except in compliance with the License.
	You may obtain a copy of the License at

		http://www.apache.org/licenses/LICENSE-2.0

	Unless required by applicable law or agreed to in writing, software
	distributed under the License is distributed on an "AS IS" BASIS,
	WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
	See the License for the specific language governing permissions and
	limitations under the License.
*/

use crate::error::Result;
use crate::error::Error;

use mio::windows::NamedPipe;

use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{FromRawHandle, IntoRawHandle, RawHandle};
use std::os::windows::prelude::AsRawHandle;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::{io};


use windows_sys::Win32::Foundation::{ERROR_NO_DATA, INVALID_HANDLE_VALUE, CloseHandle};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
};
use mio::{Events, Interest, Poll, Token};
use std::io::{Read, Write};

const SERVER: Token = Token(0);
const CLIENT: Token = Token(1);

pub struct PipeListener {
    first_instance: AtomicBool,
    address: String,
    instance_number: AtomicI32,
}

impl PipeListener {
    pub(crate) fn new(sockaddr: &str) -> Result<PipeListener> {
        Ok(PipeListener {
            first_instance: AtomicBool::new(true),
            address: sockaddr.to_string(),
            instance_number: AtomicI32::new(1),
        })
    }

    pub(crate) fn accept(
        &self,
        quit_flag: &Arc<AtomicBool>,
    ) -> std::result::Result<Option<PipeConnection>, io::Error> {
        if quit_flag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "listener shutdown for quit flag",
            ));
        }

        // Create a new pipe for every new client
        let mut namedpipe = self.new_instance().unwrap();

        let mut poll = Poll::new().unwrap();
        let mut events = Events::with_capacity(1024);
        poll.registry()
            .register(
                &mut namedpipe,
                SERVER,
                Interest::WRITABLE,
            )
            .unwrap();

        loop {
            match namedpipe.connect() {
                Ok(()) => {
                    // pipe is locked so can't use it here.
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    info!("waiting for client to connect");
                    poll.poll(&mut events, None).unwrap();
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }

        poll.registry()
            .reregister(
                &mut namedpipe,
                SERVER,
                Interest::READABLE,
            )
            .unwrap();
        let instance_num = self.instance_number.fetch_add(1, Ordering::SeqCst);
        trace!("pipe instance {} connected", instance_num);
        let pipe_instance = PipeConnection {
            named_pipe: Mutex::new(namedpipe),
            poller: Mutex::new(poll),
            instance_number: instance_num,
        };
        Ok(Some(pipe_instance))
    }

    fn new_instance(&self) -> io::Result<NamedPipe> {
        let name = OsStr::new(&self.address.as_str())
            .encode_wide()
            .chain(Some(0)) // add NULL termination
            .collect::<Vec<_>>();

        // bitwise or file_flag_first_pipe_instance with file_flag_overlapped and pipe_access_duplex
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;

        if self.first_instance.load(Ordering::SeqCst) {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            self.first_instance.swap(false, Ordering::SeqCst);
        }

        match  unsafe { CreateNamedPipeW(name.as_ptr(), open_mode, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, 65536, 65536, 0, std::ptr::null_mut())} {
            INVALID_HANDLE_VALUE => {
                return Err(io::Error::last_os_error())
            }
            h => {
                let pipe = unsafe { NamedPipe::from_raw_handle(h as RawHandle) };
                return Ok(pipe)
            },
        };
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct PipeConnection {
    named_pipe: Mutex<NamedPipe>,
    instance_number: i32,
    poller: Mutex<Poll>,
}

impl PipeConnection {
    pub(crate) fn new(h: RawHandle) -> PipeConnection {
        let mut pipe = unsafe { NamedPipe::from_raw_handle(h as RawHandle) };

        let poll = Poll::new().unwrap();

        poll.registry()
            .register(&mut pipe, CLIENT, Interest::WRITABLE | Interest::READABLE)
            .unwrap();

        PipeConnection {
            named_pipe: Mutex::new(pipe),
            poller: Mutex::new(poll),
            instance_number: 0, //todo for client scenarios
        }
    }
}

impl PipeConnection {
    pub(crate) fn id(&self) -> i32 {
        self.instance_number
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        trace!("reading from  pipe: {}", self.instance_number);

        let mut events = Events::with_capacity(1024);
        loop {
            // grabbing the lock on the poller here isn't ideal but the named pipe needs mutable access to read
            // This is ok though as read is currently blocking other threads in the server impl (only one read at a time)
            // It is also blocking until read event comes through.  This is preferable as it will not cause any cpu cycles 
            self.poller.lock().unwrap().poll(&mut events, None).unwrap();
            match self.named_pipe.lock().unwrap().read(buf) {
                Ok(0) => {
                    return Err(crate::Error::LocalClosed);
                }
                Ok(x) => {
                    return Ok(x);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) if e.raw_os_error().is_some() => {
                    return Err(crate::Error::Windows(e.raw_os_error().unwrap()))
                }
                Err(e) => {
                    trace!("Error writing to pipe: {}", e);
                    return Err(crate::Error::Others(e.to_string()));
                }
            }
        }
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        trace!("Writing to  pipe: {}", self.instance_number);
        loop {
            // grabbing the lock write to read isn't ideal
            // the named pipe needs mutable access to read
            match self.named_pipe.lock().unwrap().write(buf) {
                Ok(x) => return Ok(x),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) if e.raw_os_error() == Some(ERROR_NO_DATA as i32) => {
                    return Err(Error::Windows(e.raw_os_error().unwrap()))
                }
                Err(e) if e.raw_os_error().is_some() => {
                    return Err(Error::Windows(e.raw_os_error().unwrap()))
                }
                Err(e) => {
                    trace!("Error writing to pipe: {}", e);
                    return Err(Error::Others(e.to_string()));
                }
            }
        }
    }

    pub fn close(&self) -> Result<()> {
        let h = self.named_pipe.lock().unwrap().as_raw_handle();
        let result = unsafe { CloseHandle(h as isize) };
        match result {
            0 => Err(Error::Windows(io::Error::last_os_error().raw_os_error().unwrap())),
            _ => Ok(())
        }
    }

    pub fn shutdown(&self) -> Result<()> {
        match self.named_pipe.lock().unwrap().disconnect() {
            Ok(_) => Ok(()),
            Err(e) => Err(Error::Others(e.to_string()))
        }
    }
}

pub struct ClientConnection {
    address: String
}

impl ClientConnection {
    pub fn client_connect(sockaddr: &str) -> Result<ClientConnection> {
        Ok(ClientConnection::new(sockaddr))
    }

    pub(crate) fn new(sockaddr: &str) -> ClientConnection {
        ClientConnection {
            address: sockaddr.to_string()
        }
    }

    pub fn ready(&self) -> std::result::Result<Option<()>, io::Error> {
        Ok(Some(()))
    }

    pub fn get_pipe_connection(&self) -> PipeConnection {
        let mut opts = OpenOptions::new();
        opts.read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED);
        let file = opts.open(self.address.as_str());

        PipeConnection::new(file.unwrap().into_raw_handle())
    }

    pub fn close_receiver(&self) -> Result<()> {
        // only close from the connection object in windows
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        // only close from the connection object in windows
        Ok(())
    }
}
