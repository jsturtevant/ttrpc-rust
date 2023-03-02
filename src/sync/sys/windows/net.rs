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
use std::cell::UnsafeCell;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{IntoRawHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc};
use std::{io};

use windows_sys::Win32::Foundation::{ CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE };
use windows_sys::Win32::Storage::FileSystem::{ ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX };
use windows_sys::Win32::System::IO::{ GetOverlappedResult, OVERLAPPED };
use windows_sys::Win32::System::Pipes::{ CreateNamedPipeW, ConnectNamedPipe,DisconnectNamedPipe, PIPE_WAIT, PIPE_UNLIMITED_INSTANCES, PIPE_REJECT_REMOTE_CLIENTS };
use windows_sys::Win32::System::Threading::CreateEventW;

const PIPE_BUFFER_SIZE: u32 = 65536;
const WAIT_FOR_EVENT: i32 = 1;

pub struct PipeListener {
    first_instance: AtomicBool,
    address: String,
}

#[repr(C)]
struct Overlapped {
    inner: UnsafeCell<OVERLAPPED>,
}

impl Overlapped {
    fn new_with_event(event: isize) -> Overlapped  {        
        let mut ol = Overlapped {
            inner: UnsafeCell::new(unsafe { std::mem::zeroed() }),
        };
        ol.inner.get_mut().hEvent = event;
        ol
    }

    fn new() -> Overlapped  {
         Overlapped {
             inner: UnsafeCell::new(unsafe { std::mem::zeroed() }),
         }
     }

    fn as_mut_ptr(&self) -> *mut OVERLAPPED {
        self.inner.get()
    }
}

impl PipeListener {
    pub(crate) fn new(sockaddr: &str) -> Result<PipeListener> {
        Ok(PipeListener {
            first_instance: AtomicBool::new(true),
            address: sockaddr.to_string(),
        })
    }

    pub(crate) fn accept(&self, quit_flag: &Arc<AtomicBool>) -> std::result::Result<Option<PipeConnection>, io::Error> {
        if quit_flag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "listener shutdown for quit flag",
            ));
        }

        // Create a new pipe instance for every new client
        let np = self.new_instance().unwrap();
        let ol = Overlapped::new();

        trace!("listening for connection");
        let result = unsafe { ConnectNamedPipe(np, ol.as_mut_ptr())};
        if result != 0 {
            return Err(io::Error::last_os_error());
        }

        match io::Error::last_os_error() {
            e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(np, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
                match res {
                    0 => {
                        return Err(io::Error::last_os_error());
                    }
                    _ => {
                        Ok(Some(PipeConnection::new(np)))
                    }
                }
            }
            e if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) => {
                Ok(Some(PipeConnection::new(np)))
            }
            e => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("failed to connect pipe: {:?}", e),
                ));
            }
        }
    }

    fn new_instance(&self) -> io::Result<isize> {
        let name = OsStr::new(&self.address.as_str())
            .encode_wide()
            .chain(Some(0)) // add NULL termination
            .collect::<Vec<_>>();

        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED ;

        if self.first_instance.load(Ordering::SeqCst) {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            self.first_instance.swap(false, Ordering::SeqCst);
        }

        // null for security attributes means the handle cannot be inherited and write access is restricted to system
        // https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights
        match  unsafe { CreateNamedPipeW(name.as_ptr(), open_mode, PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, PIPE_BUFFER_SIZE, PIPE_BUFFER_SIZE, 0, std::ptr::null_mut())} {
            INVALID_HANDLE_VALUE => {
                return Err(io::Error::last_os_error())
            }
            h => {
                return Ok(h)
            },
        };
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct PipeConnection {
    named_pipe: isize,
    read_event: isize,
    write_event: isize,
}

// PipeConnection on Windows is used by both the Server and Client to read and write to the named pipe
// The named pipe is created with the overlapped flag enable the simultaneous read and write operations.
// This is required since a read and write be issued at the same time on a given named pipe instance.
//
// An event is created for the read and write operations.  When the read or write is issued
// it either returns immediately or the thread is suspended until the event is signaled when 
// the overlapped (async) operation completes and the event is triggered allow the thread to continue.
// 
// Due to the implementation of the sync Server and client there is always only one read and one write 
// operation in flight at a time so we can reuse the same event.
// 
// For more information on overlapped and events: https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult#remarks
// "It is safer to use an event object because of the confusion that can occur when multiple simultaneous overlapped operations are performed on the same file, named pipe, or communications device." 
// "In this situation, there is no way to know which operation caused the object's state to be signaled."
impl PipeConnection {
    pub(crate) fn new(h: isize) -> PipeConnection {
        trace!("creating events for thread {:?} on pipe instance {}", std::thread::current().id(), h as i32);
        let read_event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 1, std::ptr::null_mut()) };
        let write_event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 1, std::ptr::null_mut()) };
        PipeConnection {
            named_pipe: h,
            read_event: read_event,
            write_event: write_event,
        }
    }

    pub(crate) fn id(&self) -> i32 {
        self.named_pipe as i32
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        trace!("starting read for thread {:?} on pipe instance {}", std::thread::current().id(), self.named_pipe as i32);
        let ol = Overlapped::new_with_event(self.read_event);

        let len = std::cmp::min(buf.len(), u32::MAX as usize) as u32;
        let mut bytes_read= 0;
        let result = unsafe { ReadFile(self.named_pipe, buf.as_mut_ptr() as *mut _, len, &mut bytes_read,ol.as_mut_ptr()) };
        if result > 0 && bytes_read > 0 {
            // Got result no need to wait for pending read to complete
            return Ok(bytes_read as usize)
        }

        // wait for pending operation to complete (thread will be suspended until event is signaled)
        match io::Error::last_os_error() {
            ref e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(self.named_pipe, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
                match res {
                    0 => {
                        return Err(Error::Windows(io::Error::last_os_error().raw_os_error().unwrap()))
                    }
                    _ => {
                        return Ok(bytes_transfered as usize)
                    }
                }
            }
            ref e => {
                return Err(Error::Others(format!("failed to read from pipe: {:?}", e)))
            }
        }
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        trace!("starting write for thread {:?} on pipe instance {}", std::thread::current().id(), self.named_pipe as i32);
        let ol = Overlapped::new_with_event(self.write_event);
        let mut bytes_written = 0;
        let len = std::cmp::min(buf.len(), u32::MAX as usize) as u32;
        let result = unsafe { WriteFile(self.named_pipe, buf.as_ptr() as *const _,len, &mut bytes_written, ol.as_mut_ptr())};
        if result > 0 && bytes_written > 0 {
            // No need to wait for pending write to complete
            return Ok(bytes_written as usize)
        }

        // wait for pending operation to complete (thread will be suspended until event is signaled)
        match io::Error::last_os_error() {
            ref e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(self.named_pipe, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
                match res {
                    0 => {
                        return Err(Error::Windows(io::Error::last_os_error().raw_os_error().unwrap()))
                    }
                    _ => {
                        return Ok(bytes_transfered as usize)
                    }
                }
            }
            ref e => {
                return Err(Error::Others(format!("failed to write to pipe: {:?}", e)))
            }
        }
    }

    pub fn close(&self) -> Result<()> {
        close_handle(self.named_pipe)?;
        close_handle(self.read_event)?;
        close_handle(self.write_event)
    }

    pub fn shutdown(&self) -> Result<()> {
        let result = unsafe { DisconnectNamedPipe(self.named_pipe) };
        match result {
            0 => Err(Error::Windows(io::Error::last_os_error().raw_os_error().unwrap())),
            _ => Ok(()),
        }
    }
}

pub struct ClientConnection {
    address: String
}

fn close_handle(handle: isize) -> Result<()> {
    let result = unsafe { CloseHandle(handle) };
    match result {
        0 => Err(Error::Windows(io::Error::last_os_error().raw_os_error().unwrap())),
        _ => Ok(()),
    }
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
        // Windows is a "completion" based system so "readiness" isn't really applicable 
        Ok(Some(()))
    }

    pub fn get_pipe_connection(&self) -> PipeConnection {
        let mut opts = OpenOptions::new();
        opts.read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED);
        let file = opts.open(self.address.as_str());

        PipeConnection::new(file.unwrap().into_raw_handle() as isize)
    }

    pub fn close_receiver(&self) -> Result<()> {
        // close the pipe from the pipe connection
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        // close the pipe from the pipe connection
        Ok(())
    }
}
