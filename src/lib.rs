// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! This crate provides set of helpers to access [Linux Thunderbolt/USB4 bus] internals from
//! userspace. Primarily for debugging purposes but may be useful as basis for individual
//! applications as well. Most of the library uses heavily the kernel `debugfs` interface and thus
//! expects it is accessible.
//!
//! [Linux Thunderbolt/USB4 bus]: https://docs.kernel.org/admin-guide/thunderbolt.html

mod cros;
mod device;

pub use device::*;

pub mod debugfs;
pub mod margining;
pub mod monitor;
pub mod trace;
pub mod typec;
pub mod usb4;
pub mod util;
