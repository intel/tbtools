// Thunderbolt/USB4 debug tools
//
// Cros EC Flags and structure layouts are taken from upstream Chromium
// ectool source code
//   https://chromium.googlesource.com/chromiumos/platform/ec/util/ectool.cc
//   https://chromium.googlesource.com/chromiumos/platform/ec/include/ec_commands.h
// which are covered by BSD-style license with the following copyright:
//   Copyright 2014 The ChromiumOS Authors
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Simple ChromeOS embedded controller access implementation to be used for alternate mode
//! switching.

use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Result};
use std::mem;
use std::os::fd::AsRawFd;
use std::ptr;

use crate::{
    device::Address,
    typec::{AltMode, AltModeControl},
};

mod ioctl {
    use nix::ioctl_readwrite;

    pub(crate) const CROS_EC_MAX_DATA_SIZE: usize = 64 * 1024;

    const CROS_EC_DEV_IOC: u8 = 0xec;

    #[repr(C)]
    pub(crate) struct HelloParams {
        pub in_data: u32,
    }

    #[repr(C)]
    pub(crate) struct HelloResponse {
        pub out_data: u32,
    }

    #[repr(C)]
    pub(crate) struct UsbPdMuxInfoParams {
        pub port: u8,
    }

    #[repr(C)]
    pub(crate) struct UsbPdMuxInfoResponse {
        pub flags: u8,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(crate) struct UsbMuxSet {
        pub mux_index: u8,
        pub mux_flags: u8,
    }

    #[repr(C)]
    pub(crate) union TypecParam {
        pub mode_to_enter: u8,
        pub mux_params: UsbMuxSet,
        pub placeholder: [u8; 128],
    }

    #[repr(C)]
    pub(crate) struct TypecControl {
        pub port: u8,
        pub command: u8,
        pub reserved: u16,
        pub param: TypecParam,
    }

    #[repr(C)]
    pub(crate) struct EcCommand {
        pub version: u32,
        pub command: u32,
        pub outsize: u32,
        pub insize: u32,
        pub result: u32,
        pub data: [u8; CROS_EC_MAX_DATA_SIZE],
    }

    ioctl_readwrite!(ec_command, CROS_EC_DEV_IOC, 0, EcCommand);
}

// Convert from USB4 adapter to ChromeOS type-C port number.
fn address_to_port(address: &Address) -> Result<u8> {
    if let Address::Adapter {
        domain: _,
        route,
        adapter,
    } = address
    {
        // Only supported for host router.
        if *route != 0 {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        match adapter {
            1 => return Ok(0),
            3 => return Ok(1),
            _ => (),
        }
    };

    Err(io::Error::from(ErrorKind::InvalidData))
}

fn flags_to_altmode(flags: u8) -> AltMode {
    if (flags & USB_PD_MUX_DP_ENABLED) == USB_PD_MUX_DP_ENABLED {
        AltMode::DisplayPort
    } else if (flags & USB_PD_MUX_SAFE_MODE) == USB_PD_MUX_SAFE_MODE {
        AltMode::Safe
    } else if (flags & USB_PD_MUX_TBT_MODE) == USB_PD_MUX_TBT_MODE {
        AltMode::Thunderbolt
    } else if (flags & USB_PD_MUX_USB4_MODE) == USB_PD_MUX_USB4_MODE {
        AltMode::Usb4
    } else {
        AltMode::Usb
    }
}

fn altmode_to_flags(am: &AltMode) -> u8 {
    match am {
        AltMode::Safe => USB_PD_MUX_SAFE_MODE,
        AltMode::Usb => USB_PD_MUX_USB_ENABLED,
        AltMode::Usb4 => USB_PD_MUX_USB4_MODE,
        AltMode::DisplayPort => USB_PD_MUX_DP_ENABLED,
        AltMode::Thunderbolt => USB_PD_MUX_TBT_MODE,
    }
}

// Commands sent to the EC.
enum Command {
    Hello = 0x0001,
    UsbPdMuxInfo = 0x011a,
    TypecControl = 0x0132,
}

const CROS_EC_DEV: &str = "/dev/cros_ec";

// Flags used with mux.
const USB_PD_MUX_USB_ENABLED: u8 = 1 << 0;
const USB_PD_MUX_DP_ENABLED: u8 = 1 << 1;
const USB_PD_MUX_SAFE_MODE: u8 = 1 << 5;
const USB_PD_MUX_TBT_MODE: u8 = 1 << 6;
const USB_PD_MUX_USB4_MODE: u8 = 1 << 7;
// Commands.
const TYPEC_CONTROL_COMMAND_EXIT_MODES: u8 = 0;
const TYPEC_CONTROL_COMMAND_ENTER_MODE: u8 = 2;
const TYPEC_CONTROL_COMMAND_USB_MUX_SET: u8 = 4;
// Modes.
const TYPEC_MODE_DP: u8 = 0;
const TYPEC_MODE_TBT: u8 = 1;
const TYPEC_MODE_USB4: u8 = 2;

/// ChromeOS embedded controller.
///
/// This represents a communications channel to the ChromeOS embedded controller firmware.
pub(crate) struct Ec {
    dev: File,
}

impl Ec {
    /// Opens the EC character device and returns `Ec` that can be used to send commands through
    /// it.
    pub(crate) fn open() -> Result<Self> {
        let dev = OpenOptions::new()
            .read(true)
            .write(true)
            .open(CROS_EC_DEV)?;

        let ec = Self { dev };

        // Check that it is running.
        ec.hello()?;

        Ok(ec)
    }

    fn command<T, U>(&self, command: Command, request: &T, response: &mut U) -> Result<()> {
        let fd = self.dev.as_raw_fd();

        let outsize = mem::size_of::<T>();
        let insize = mem::size_of::<U>();

        if outsize > ioctl::CROS_EC_MAX_DATA_SIZE || insize > ioctl::CROS_EC_MAX_DATA_SIZE {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }

        let mut cmd = ioctl::EcCommand {
            version: 0,
            command: command as u32,
            outsize: outsize as u32,
            insize: insize as u32,
            result: 0xff,
            data: [0; ioctl::CROS_EC_MAX_DATA_SIZE],
        };

        // Take the raw pointers.
        let data = ptr::addr_of_mut!(cmd.data) as *mut u8;
        let source = request as *const T as *const u8;
        let dest = response as *mut U as *mut u8;

        // SAFETY: `source` is not overlapping  `data` and it is always smaller than equal to data
        // size (`ioctl::CROS_EC_MAX_DATA_SIZE`). Same is true for `dest` and `data`.
        // Both operands are accessed as bytes so their alignment does not matter.
        //
        // Calling the `ioctl::ec_command()` is just an FFI call.
        unsafe {
            // Copy raw bytes from request into cmd.data.
            ptr::copy(source, data, outsize);
            // Run the ioctl()
            ioctl::ec_command(fd, &mut cmd)?;
            // Copy back raw bytes from cmd.data into response.
            ptr::copy(data, dest, insize);
        }

        Ok(())
    }

    fn hello(&self) -> Result<()> {
        let params = ioctl::HelloParams {
            in_data: 0xa0b0c0d0,
        };
        let mut response = ioctl::HelloResponse { out_data: 0 };

        self.command(Command::Hello, &params, &mut response)?;

        if response.out_data != 0xa1b2c3d4 {
            eprintln!(
                "unexpected response {:x} expected {:x}",
                response.out_data, 0xa1b2c3d4u32
            );
            return Err(io::Error::from(ErrorKind::InvalidData));
        }

        Ok(())
    }

    fn mux_info(&self, port: u8) -> Result<u8> {
        let p = ioctl::UsbPdMuxInfoParams { port };
        let mut r = ioctl::UsbPdMuxInfoResponse { flags: 0 };

        self.command(Command::UsbPdMuxInfo, &p, &mut r)?;

        Ok(r.flags)
    }

    fn set_mux_mode(&self, port: u8, flags: u8) -> Result<()> {
        let control = ioctl::TypecControl {
            port,
            command: TYPEC_CONTROL_COMMAND_USB_MUX_SET,
            reserved: 0,
            param: ioctl::TypecParam {
                mux_params: ioctl::UsbMuxSet {
                    mux_index: 0,
                    mux_flags: flags,
                },
            },
        };
        let mut response = [0u8; ioctl::CROS_EC_MAX_DATA_SIZE];

        self.command(Command::TypecControl, &control, &mut response)?;

        Ok(())
    }

    fn exit_modes(&self, port: u8) -> Result<()> {
        let control = ioctl::TypecControl {
            port,
            command: TYPEC_CONTROL_COMMAND_EXIT_MODES,
            reserved: 0,
            param: ioctl::TypecParam {
                placeholder: [0; 128],
            },
        };
        let mut response = [0u8; ioctl::CROS_EC_MAX_DATA_SIZE];

        self.command(Command::TypecControl, &control, &mut response)?;

        Ok(())
    }

    fn enter_mode(&self, port: u8, mode: u8) -> Result<()> {
        let control = ioctl::TypecControl {
            port,
            command: TYPEC_CONTROL_COMMAND_ENTER_MODE,
            reserved: 0,
            param: ioctl::TypecParam {
                mode_to_enter: mode,
            },
        };
        let mut response = [0u8; ioctl::CROS_EC_MAX_DATA_SIZE];

        self.command(Command::TypecControl, &control, &mut response)?;

        Ok(())
    }
}

impl AltModeControl for Ec {
    fn current_mode(&self, address: &Address) -> Result<AltMode> {
        let port = address_to_port(address)?;
        let flags = self.mux_info(port)?;

        Ok(flags_to_altmode(flags))
    }

    fn enter_mode(&self, address: &Address, mode: &AltMode) -> Result<()> {
        let current_mode = self.current_mode(address)?;

        if current_mode == *mode {
            return Ok(());
        }

        let port = address_to_port(address)?;
        let flags = altmode_to_flags(mode);

        // Changing modes so put mux first to safe mode.
        self.set_mux_mode(port, USB_PD_MUX_SAFE_MODE)?;

        match mode {
            AltMode::Safe => return Ok(()),
            AltMode::Usb => {
                self.exit_modes(port)?;
                self.set_mux_mode(port, flags)?;
            }
            AltMode::Usb4 => {
                self.enter_mode(port, TYPEC_MODE_USB4)?;
                self.set_mux_mode(port, flags)?;
            }
            AltMode::DisplayPort => {
                self.enter_mode(port, TYPEC_MODE_DP)?;
                self.set_mux_mode(port, flags)?;
            }
            AltMode::Thunderbolt => {
                self.enter_mode(port, TYPEC_MODE_TBT)?;
                self.set_mux_mode(port, flags)?;
            }
        }

        Ok(())
    }
}
