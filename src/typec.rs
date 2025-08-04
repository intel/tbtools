// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! USB Type-C alternate mode control helpers.
//!
//! This can be used to put a USB Type-C port into native or alternate mode. Specifically useful in
//! ChromeOS where this is done through the kernel.

use std::fmt;
use std::io::{self, ErrorKind, Result};

// Use here ValueEnum for AltMode to avoid duplicating the enum in tbpd.rs.
use clap::ValueEnum;

use crate::{Address, cros};

/// Alternate modes. Even though `Safe`, `Usb` and `Usb4` are not alternate modes in USB Power
/// Delivery specification we specify them as such. This allows the caller to pass desired mode to
/// the controller to enter without need to exit certain modes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
pub enum AltMode {
    /// Safe mode
    Safe,
    /// USB 2.x and USB 3.x native modes
    Usb,
    /// USB4 native mode
    Usb4,
    /// DisplayPort alternate mode
    DisplayPort,
    /// Thunderbolt 3 alternate mode
    Thunderbolt,
}

impl fmt::Display for AltMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Safe => write!(f, "Safe"),
            Self::Usb => write!(f, "USB"),
            Self::Usb4 => write!(f, "USB4"),
            Self::DisplayPort => write!(f, "DisplayPort"),
            Self::Thunderbolt => write!(f, "Thunderbolt"),
        }
    }
}

/// Type-C alternate mode controllers implement this trait
pub trait AltModeControl {
    /// Returns current mode of a USB4 port.
    fn current_mode(&self, address: &Address) -> Result<AltMode>;
    /// Enters desired alternate mode.
    fn enter_mode(&self, address: &Address, mode: &AltMode) -> Result<()>;
}

/// Returns USB4 alternate mode controller for the current system.
///
/// # Examples
/// ```no_run
/// # use std::io;
/// use tbtools::Address;
/// use tbtools::typec::{self, AltMode, AltModeControl};
///
/// # fn main() -> io::Result<()> {
/// // Get access to the controller.
/// let ac = typec::controller()?;
///
/// // Put host router first USB4 port into USB4 mode.
/// let address = Address::Adapter { domain: 0, route: 0, adapter: 1 };
/// ac.enter_mode(&address, &AltMode::Usb4)?;
/// # Ok(())
/// # }
/// ```
pub fn controller() -> Result<impl AltModeControl> {
    let result = cros::Ec::open();
    match result {
        Ok(ec) => Ok(ec),
        Err(err) => {
            if err.kind() == ErrorKind::NotFound {
                // TODO: Add standard connector class support
                Err(io::Error::from(ErrorKind::Unsupported))
            } else {
                Err(err)
            }
        }
    }
}
