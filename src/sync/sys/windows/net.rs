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
use std::io;
use std::os::windows::prelude::RawHandle;
use std::sync::{Arc};
use std::sync::atomic::{AtomicBool, Ordering};



pub struct PipeListener {

}

impl PipeListener {
    pub(crate) fn new(sockaddr: &str) -> Result<PipeListener> {
     
        Ok(PipeListener {
           
        })
    }
   
    pub(crate) fn accept( &self, quit_flag: &Arc<AtomicBool>) ->  std::result::Result<Option<PipeConnection>, io::Error> {
        if quit_flag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(io::ErrorKind::Other, "listener shutdown for quit flag"));
        }
        
        //todo

        if quit_flag.load(Ordering::SeqCst) {
            info!("listener shutdown for quit flag");
            return Err(io::Error::new(io::ErrorKind::Other, "listener shutdown for quit flag"));
        }
     
        Ok(Some(PipeConnection { }))
    }

    pub fn close(&self) -> Result<()> {
       
        Ok(())
    }
}


pub struct PipeConnection {

}

impl PipeConnection {
    pub(crate) fn new() -> PipeConnection {
        PipeConnection {  }
    }
}

impl PipeConnection {
    pub(crate) fn id(&self) -> i32 {
        0
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        Ok((0))
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok((0))
        
    }

    pub fn close(&self) -> Result<()> {
       Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

pub struct ClientConnection {

}

impl ClientConnection {
    pub fn client_connect(sockaddr: &str)-> Result<ClientConnection>   {

        Ok(ClientConnection::new())
    }

    pub(crate) fn new() -> ClientConnection {
       


        ClientConnection { 

        }
    }

    pub fn ready(&self) -> std::result::Result<Option<()>, io::Error> {
        Ok(Some(()))
    }

    pub fn get_pipe_connection(&self) -> PipeConnection {
        PipeConnection::new()
    }

    pub fn close_receiver(&self) -> Result<()> {
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        Ok(())
    }
        
    
}

