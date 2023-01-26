// Copyright (c) 2020 Ant Financial
//
// SPDX-License-Identifier: Apache-2.0
//
#[cfg(feature = "async")]
pub mod asynchronous;
#[cfg(feature = "async")]
pub use asynchronous as r#async;
pub mod sync;