// Write Thunderbolt/USB4 config spaces
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::io::{self, ErrorKind};
use std::process;

use clap::Parser;
use nix::unistd::Uid;

use tbtools::{
    debugfs::{self, BitFields},
    util, Address, Device,
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Write Thunderbolt/USB4 config spaces", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device
    #[arg(value_parser = util::parse_route, short, long)]
    route: u64,
    /// Adapter number if accessing adapters
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..64))]
    adapter: Option<u8>,
    /// Select path config space of an adapter
    #[arg(short, long)]
    path: bool,
    /// Select counters config space of an adapter
    #[arg(short, long)]
    counters: bool,
    /// One or more values to write in format offset=value or name[.field]=value
    values: Vec<String>,
}

fn write_router(device: &mut Device, values: &Vec<(String, u32)>) -> io::Result<()> {
    device.read_registers()?;

    for value in values {
        match util::parse_hex::<u16>(&value.0) {
            Some(offset) => {
                if let Some(reg) = device.register_by_offset_mut(offset) {
                    reg.set_value(value.1);
                } else {
                    eprintln!("Warning: invalid offset {}!", offset);
                }
            }
            None => {
                let name: Vec<_> = value.0.split('.').collect();

                if let Some(reg) = device.register_by_name_mut(name[0]) {
                    if name.len() > 1 {
                        if reg.has_field(name[1]) {
                            reg.set_field(name[1], value.1);
                        } else {
                            eprintln!("Warning: field name {} not found!", &name[1]);
                        }
                    } else {
                        reg.set_value(value.1);
                    }
                } else {
                    eprintln!("Warning: register name {} not found!", &name[0]);
                }
            }
        }
    }

    device.write_changed()
}

fn write_adapter(
    device: &mut Device,
    adapter: u8,
    values: &Vec<(String, u32)>,
    args: &Args,
) -> io::Result<()> {
    device.read_adapters()?;

    if let Some(adapter) = device.adapter_mut(adapter) {
        if args.counters {
            adapter.clear_counters()?;
        } else {
            if args.path {
                // Adapter registers are already read but read path registers now if user is
                // writing to them.
                adapter.read_paths()?;
            }

            for value in values {
                match util::parse_hex::<u16>(&value.0) {
                    Some(offset) => {
                        let reg = if args.path {
                            adapter.path_register_by_offset_mut(offset)
                        } else {
                            adapter.register_by_offset_mut(offset)
                        };

                        if let Some(reg) = reg {
                            reg.set_value(value.1);
                        } else {
                            eprintln!("Warning: invalid offset {}!", offset);
                        }
                    }

                    None => {
                        if args.path {
                            eprintln!("Warning: path registers do not have names!");
                        } else {
                            let name: Vec<_> = value.0.split('.').collect();

                            if let Some(reg) = adapter.register_by_name_mut(name[0]) {
                                if name.len() > 1 {
                                    if reg.has_field(name[1]) {
                                        reg.set_field(name[1], value.1);
                                    } else {
                                        eprintln!("Warning: field name {} not found!", &name[1]);
                                    }
                                } else {
                                    reg.set_value(value.1);
                                }
                            } else {
                                eprintln!("Warning: register name {} not found!", &name[0]);
                            }
                        }
                    }
                }
            }
        }

        adapter.write_changed()?;
    } else {
        eprintln!("Error: adapter {} not found!", adapter);
        process::exit(1);
    }

    Ok(())
}

fn parse_one_value(s: &str) -> (String, u32) {
    let values: Vec<&str> = s.split('=').collect();

    if values.len() != 2 {
        eprintln!("Error: offset=value or name=value pairs expected!");
        process::exit(1);
    }

    let value = util::parse_hex::<u32>(values[1]).unwrap_or_else(|| {
        eprintln!("Error: value needs to be numeric!");
        process::exit(1);
    });

    (String::from(values[0]), value)
}

fn write(args: &Args) -> io::Result<()> {
    let address = Address::Router {
        domain: args.domain,
        route: args.route,
    };
    let mut device = match tbtools::find_device(&address)? {
        Some(device) => device,
        None => {
            eprintln!("Error: No such device found!");
            process::exit(1);
        }
    };

    if !device.registers_writable() {
        eprintln!(
            "Error: Device registers are not writeable! You may need to set
CONFIG_USB4_DEBUGFS_WRITE=y in your kernel .config."
        );
        process::exit(1);
    }

    // Extract name/offset=value pairs from the arguments.
    let values: Vec<(String, u32)> = args.values.iter().map(|s| parse_one_value(s)).collect();

    if !values.is_empty() {
        if let Some(adapter) = args.adapter {
            write_adapter(&mut device, adapter, &values, args)?
        } else {
            write_router(&mut device, &values)?;
        }
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

    if args.values.is_empty() {
        eprintln!("Error: Missing parameters!");
        process::exit(1);
    }

    if !Uid::current().is_root() {
        eprintln!("Error: debugfs access requires root permissions!");
        process::exit(1);
    }

    if let Err(err) = debugfs::mount() {
        eprintln!("Error: failed to mount debugfs: {}", err);
        process::exit(1);
    }

    if let Err(err) = write(&args) {
        eprintln!("Error: {}", err);
        if err.kind() == ErrorKind::Unsupported {
            eprintln!("Device does not support register access");
        }
        process::exit(1);
    }
}
