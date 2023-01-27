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
use crate::sync;
use mio::windows::NamedPipe;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::{io, thread};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, RawHandle};
use std::sync::atomic::{AtomicBool, Ordering, AtomicI32};
use std::sync::{Arc, Mutex};
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
const CLIENT: Token = Token(1);

pub struct PipeListener {
    firstInstance: AtomicBool,
    address: String,
    instanceNumber: AtomicI32,
}

impl PipeListener {
    pub(crate) fn new(sockaddr: &str) -> Result<PipeListener> {
        Ok(PipeListener {
            firstInstance: AtomicBool::new(true),
            address: sockaddr.to_string(),
            instanceNumber:  AtomicI32::new(1)
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

        let pipe_instance = self.new_instance().unwrap();

    
        println!("waiting for connection....");
        loop {
            match pipe_instance.namedPipe.lock().unwrap().connect() {
                Ok(()) => {
                    println!("Server Connected!");
                    trace!("handed off pipe instance : {}", pipe_instance.id());
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    {
                        continue;
                    }
                }
                Err(e) => {
                    println!("Error connecting to pipe: {}", e);
                    return Err(e);
                }
            }
        }

        return Ok(Some(pipe_instance))
    }

    fn new_instance(& self) -> io::Result<PipeConnection> {
        let name = OsStr::new(&self.address.as_str())
            .encode_wide()
            .chain(Some(0)) // add NULL termination
            .collect::<Vec<_>>();

        // bitwise or file_flag_first_pipe_instance with file_flag_overlapped and pipe_access_duplex
        let mut openmode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;

        if self.firstInstance.load(Ordering::SeqCst) {
            openmode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            self.firstInstance.swap(false, Ordering::SeqCst);
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
            let instance_num = self.instanceNumber.fetch_add(1, Ordering::SeqCst);
            trace!("created pipe instance : {}", instance_num);

            let mut pipe = unsafe { NamedPipe::from_raw_handle(h as RawHandle)};

            let poll = Poll::new().unwrap();
            {
                poll
                .registry()
                .register(&mut pipe, SERVER, Interest::WRITABLE | Interest::READABLE)
                .unwrap();
            }
       
            let h = thread::spawn(move || {
                let mut poller = poll;
                let mut events = Events::with_capacity(1024);
                loop {
                    poller.poll(&mut events, None).unwrap();
                }
            });

            Ok(PipeConnection {
                namedPipe:  Mutex::new(pipe),
                instance_number: instance_num })
        }
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct PipeConnection {
    namedPipe: Mutex<NamedPipe>,
    instance_number: i32,
}

unsafe impl Send for PipeConnection {}
unsafe impl Sync for PipeConnection {}

impl PipeConnection {
    pub(crate) fn new(h: RawHandle) -> PipeConnection {
        let mut pipe = unsafe { NamedPipe::from_raw_handle(h as RawHandle)};

        let poll = Poll::new().unwrap();
        {
            poll
            .registry()
            .register(&mut pipe, CLIENT, Interest::WRITABLE | Interest::READABLE)
            .unwrap();
        }
       
        let h = thread::spawn(move || {
            let mut poller = poll;
            let mut events = Events::with_capacity(1024);
            loop {
                poller.poll(&mut events, None).unwrap();
            }
        });

        PipeConnection {
            namedPipe: Mutex::new(pipe),
            instance_number: 0, //todo
        }
    }
}

impl PipeConnection {
    pub(crate) fn id(&self) -> i32 {
        self.instance_number
    }

    pub fn read(& self, buf: &mut [u8]) -> Result<usize> {
        trace!("reading from  pipe: {}", self.instance_number);
        
        loop {
            //trace!("waiting for msg: {}", self.instance_number);
            match self.namedPipe.lock().unwrap().read(buf) {
                Ok(0) => {
                    return Err(crate::Error::LocalClosed);
                }
                Ok(x) => {
                    //print!("read: {:?}", std::str::from_utf8(&buf));
                    return Ok(x);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue 
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

    pub fn write(& self, buf: &[u8]) -> Result<usize> {
        trace!("Writing to  pipe: {}", self.instance_number);
        
        loop {
            match self.namedPipe.lock().unwrap().write(buf) {
                Ok(x) => return {           
                    Ok(x)
                },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    continue;
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
        
        pipe
    }

    pub fn close_receiver(&self) -> Result<()> {
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}
