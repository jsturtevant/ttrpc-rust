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

const PIPE_BUFFER_SIZE:u32 = 65536;
const WAIT_FOR_EVENT: i32 = 1;

struct NamedPipe(isize);

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

        // Create a new pipe for every new client
        let np = self.new_instance().unwrap();
        let ol= Overlapped::new();

        trace!("listening for connection");
        let result = unsafe { ConnectNamedPipe(np.0, ol.as_mut_ptr())};
        if result != 0 {
            return Err(io::Error::last_os_error());
        }

        match io::Error::last_os_error() {
            e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(np.0, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
                match res {
                    0 => {
                        return Err(io::Error::last_os_error());
                    }
                    _ => {
                        Ok(Some(PipeConnection::new(np.0)))
                    }
                }
            }
            e if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) => {
                Ok(Some(PipeConnection::new(np.0)))
            }
            e => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("failed to connect pipe: {:?}", e),
                ));
            }
        }
    }

    fn new_instance(&self) -> io::Result<NamedPipe> {
        let name = OsStr::new(&self.address.as_str())
            .encode_wide()
            .chain(Some(0)) // add NULL termination
            .collect::<Vec<_>>();

        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED ;

        if self.first_instance.load(Ordering::SeqCst) {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            self.first_instance.swap(false, Ordering::SeqCst);
        }

        match  unsafe { CreateNamedPipeW(name.as_ptr(), open_mode, PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, PIPE_UNLIMITED_INSTANCES, PIPE_BUFFER_SIZE, PIPE_BUFFER_SIZE, 0, std::ptr::null_mut())} {
            INVALID_HANDLE_VALUE => {
                return Err(io::Error::last_os_error())
            }
            h => {
                return Ok(NamedPipe(h))
            },
        };
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
}

pub struct PipeConnection {
    named_pipe: NamedPipe,
    read_event: isize,
    write_event: isize,
}

impl PipeConnection {
    pub(crate) fn new(h: isize) -> PipeConnection {
        // create and event to wait on 
        // https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getoverlappedresult#remarks
        // "It is safer to use an event object because of the confusion that can occur when multiple simultaneous overlapped operations are performed on the same file, named pipe, or communications device." 
        // "In this situation, there is no way to know which operation caused the object's state to be signaled."
        let read_name = OsStr::new(format!("read-{}-{:?}", h as i32,std::thread::current().id()).as_str())
        .encode_wide()
        .chain(Some(0)) // add NULL termination
        .collect::<Vec<_>>();
        let write_name = OsStr::new(format!("write-{}-{:?}", h as i32,std::thread::current().id()).as_str())
        .encode_wide()
        .chain(Some(0)) // add NULL termination
        .collect::<Vec<_>>();
        let read_event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 1, read_name.as_ptr()) };
        let write_event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 1, write_name.as_ptr()) };
        PipeConnection {
            named_pipe: NamedPipe(h),
            read_event: read_event,
            write_event: write_event,
        }
    }
}

impl PipeConnection {
    pub(crate) fn id(&self) -> i32 {
        self.named_pipe.0 as i32
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let ol = Overlapped::new_with_event(self.read_event);

        let len = std::cmp::min(buf.len(), u32::MAX as usize) as u32;
        let mut bytes_read= 0;
        let result = unsafe { ReadFile(self.named_pipe.0, buf.as_mut_ptr() as *mut _, len, &mut bytes_read,ol.as_mut_ptr()) };
        if result > 0 && bytes_read > 0 {
            // Got result no need to wait for pending read to complete
            return Ok(bytes_read as usize)
        }

        match io::Error::last_os_error() {
            ref e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(self.named_pipe.0, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
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
        let ol = Overlapped::new_with_event(self.write_event);
        let mut bytes_written= 0;
        let len = std::cmp::min(buf.len(), u32::MAX as usize) as u32;
        let result = unsafe { WriteFile(self.named_pipe.0, buf.as_ptr() as *const _,len, &mut bytes_written, ol.as_mut_ptr())};
        if result > 0 && bytes_written > 0 {
            // No need to wait for pending write to complete
            return Ok(bytes_written as usize)
        }

        match io::Error::last_os_error() {
            ref e if e.raw_os_error() == Some(ERROR_IO_PENDING as i32) => {
                let mut bytes_transfered = 0;
                let res = unsafe {GetOverlappedResult(self.named_pipe.0, ol.as_mut_ptr(), &mut bytes_transfered, WAIT_FOR_EVENT) };
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
        close_handle(self.named_pipe.0)?;
        close_handle(self.read_event)?;
        close_handle(self.write_event)
    }

    pub fn shutdown(&self) -> Result<()> {
        let result = unsafe { DisconnectNamedPipe(self.named_pipe.0) };
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
