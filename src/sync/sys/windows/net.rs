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
use mio::windows::NamedPipe;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, RawHandle};
use std::sync::atomic::{AtomicBool, Ordering, AtomicI32};
use std::sync::Arc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{ERROR_NO_DATA, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
};

use mio::{Events, Interest, Poll, Token};
use std::io::{Read, Write};

const SERVER: Token = Token(0);

pub struct PipeListener {
    firstInstance: Option<bool>,
    address: String,
    instanceNumber: AtomicI32,
}

impl PipeListener {
    pub(crate) fn new(sockaddr: &str) -> Result<PipeListener> {
        Ok(PipeListener {
            firstInstance: None,
            address: sockaddr.to_string(),
            instanceNumber:  AtomicI32::new(1)
        })
    }

    pub(crate) fn accept(
        &mut self,
        quit_flag: &Arc<AtomicBool>,
    ) -> std::result::Result<Option<PipeConnection>, io::Error> {
        if quit_flag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "listener shutdown for quit flag",
            ));
        }

        let mut pipe = self.new_instance().unwrap();

        pipe.poll
            .registry()
            .register(&mut pipe.namedPipe, SERVER, Interest::WRITABLE)
            .unwrap();

        println!("waiting for connection....");
        loop {
            match pipe.namedPipe.connect() {
                Ok(()) => {
                    println!("Server Connected!");
                    trace!("handed off pipe instance : {}", pipe.id());
                    return Ok(Some(pipe));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    pipe.poll
                        .registry()
                        .reregister(&mut pipe.namedPipe, SERVER, Interest::WRITABLE)
                        .unwrap();

                    let mut events = Events::with_capacity(1024);
                    pipe.poll.poll(&mut events, None).unwrap();
                }
                Err(e) => {
                    println!("Error connecting to pipe: {}", e);
                    return Err(e);
                }
            }
        }
    }

    fn new_instance(&mut self) -> io::Result<PipeConnection> {
        let name = OsStr::new(&self.address.as_str())
            .encode_wide()
            .chain(Some(0)) // add NULL termination
            .collect::<Vec<_>>();

        // bitwise or file_flag_first_pipe_instance with file_flag_overlapped and pipe_access_duplex
        let mut openmode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;

        match self.firstInstance {
            Some(_) => {}
            None => {
                self.firstInstance = Some(true);
                openmode |= FILE_FLAG_FIRST_PIPE_INSTANCE
            }
        }

        // Safety: syscall
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                openmode,
                PIPE_TYPE_BYTE,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                std::ptr::null_mut(), // todo set this on first instance
            )
        };

        if h == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            // Safety: nothing actually unsafe about this. The trait fn includes
            // `unsafe`.
            let np = unsafe { NamedPipe::from_raw_handle(h as RawHandle) };

            let instance_num = self.instanceNumber.fetch_add(1, Ordering::SeqCst);
            trace!("created pipe instance : {}", instance_num);
            Ok(PipeConnection {
                namedPipe: np,
                poll: Poll::new().unwrap(),
                instance_number: instance_num
            })
        }
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct PipeConnection {
    namedPipe: NamedPipe,
    poll: Poll,
    instance_number: i32,
}

impl PipeConnection {
    pub(crate) fn new(h: RawHandle) -> PipeConnection {
        let np = unsafe { NamedPipe::from_raw_handle(h as RawHandle) };
        PipeConnection {
            namedPipe: np,
            poll: Poll::new().unwrap(),
            instance_number: 0, //todo
        }
    }
}

impl PipeConnection {
    pub(crate) fn id(&self) -> i32 {
        self.instance_number
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        trace!("reading from  pipe: {}", self.instance_number);
        
        self.poll
            .registry()
            .reregister(&mut self.namedPipe, SERVER, Interest::READABLE)
            .unwrap();

        // let mut events = Events::with_capacity(1024);
        // self.poll.poll(&mut events, Some(Duration::from_secs(1)) ).unwrap();


        trace!("waiting for msg: {}", self.instance_number);
        match self.namedPipe.read(buf) {
            Ok(0) => {
                return Err(crate::Error::LocalClosed);
            }
            Ok(x) => {
                print!("read: {:?}", std::str::from_utf8(&buf));

                return Ok(x);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                trace!("no  for msg: {}", self.instance_number);
                //self.poll.poll(&mut events, Some(Duration::from_secs(1)) ).unwrap();
                return Ok(0)
            }
            Err(e) if e.raw_os_error() != None => {
                return Err(crate::Error::Windows(e.raw_os_error().unwrap()))
            }
            Err(e) => {
                trace!("Error writing to pipe: {}", e);
                return Err(crate::Error::Others(e.to_string()));
            }
        }
        
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        trace!("Writing to  pipe: {}", self.instance_number);
        self.poll
            .registry()
            .reregister(&mut self.namedPipe, SERVER, Interest::WRITABLE)
            .unwrap();

        loop {
            match self.namedPipe.write(buf) {
                Ok(x) => return Ok(x),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    let mut events = Events::with_capacity(1024);
                    self.poll.poll(&mut events, None).unwrap();
                }
                Err(e) if e.raw_os_error() == Some(ERROR_NO_DATA as i32) => {
                    return Err(crate::Error::Windows(e.raw_os_error().unwrap()))
                }
                Err(e) if e.raw_os_error() != None => {
                    return Err(crate::Error::Windows(e.raw_os_error().unwrap()))
                }
                Err(e) => {
                    trace!("Error writing to pipe: {}", e);
                    return Err(crate::Error::Others(e.to_string()));
                }
            }
        }
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct ClientConnection {}

impl ClientConnection {
    pub fn client_connect(sockaddr: &str) -> Result<ClientConnection> {
        Ok(ClientConnection::new())
    }

    pub(crate) fn new() -> ClientConnection {
        ClientConnection {}
    }

    pub fn ready(&self) -> std::result::Result<Option<()>, io::Error> {
        Ok(Some(()))
    }

    pub fn get_pipe_connection(&self) -> PipeConnection {
        let mut opts = OpenOptions::new();
        opts.read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED);
        let file = opts.open(r"\\.\pipe\mio-named-pipe-test");
        let mut pipe = PipeConnection::new(file.unwrap().into_raw_handle());

        pipe.poll
            .registry()
            .register(&mut pipe.namedPipe, SERVER, Interest::WRITABLE)
            .unwrap();
        
        pipe
    }

    pub fn close_receiver(&self) -> Result<()> {
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}
