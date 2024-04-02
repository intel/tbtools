// Read Thunderbolt/USB4 config spaces
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
#[command(about = "Read Thunderbolt/USB4 config spaces", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device
    #[arg(value_parser = util::parse_route, short, long)]
    route: u64,
    /// Adapter number if accessing adapters
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..64))]
    adapter: Option<u16>,
    /// Select path config space of an adapter
    #[arg(short, long)]
    path: bool,
    /// Select counters config space of an adapter
    #[arg(short, long)]
    counters: bool,
    /// Output in binary instead of hex
    #[arg(short = 'B', long, group = "output")]
    binary: bool,
    /// Output in decimal instead of hex
    #[arg(short = 'D', long, group = "output")]
    decimal: bool,
    /// One or more registers to read in format offset or name[.field]
    regs: Vec<String>,
}

fn dump_value(value: u32, args: &Args) {
    if args.binary {
        println!("{:#b}", value);
    } else if args.decimal {
        println!("{}", value);
    } else {
        println!("{:#x}", value);
    }
}

fn read_router(device: &mut Device, args: &Args) -> io::Result<()> {
    for reg in &args.regs {
        match util::parse_hex::<u16>(reg) {
            Some(offset) => {
                if let Some(reg) = device.register_by_offset_mut(offset) {
                    dump_value(reg.value(), args);
                } else {
                    eprintln!("Warning: invalid offset {}!", offset);
                }
            }
            None => {
                let name: Vec<_> = reg.split('.').collect();

                if let Some(reg) = device.register_by_name_mut(name[0]) {
                    if name.len() > 1 {
                        if reg.has_field(name[1]) {
                            dump_value(reg.field(name[1]), args);
                        } else {
                            eprintln!("Warning: field name {} not found!", &name[1]);
                        }
                    } else {
                        dump_value(reg.value(), args);
                    }
                } else {
                    eprintln!("Warning: register name {} not found!", &name[0]);
                }
            }
        }
    }

    Ok(())
}

fn read_adapter(device: &mut Device, adapter: u16, args: &Args) -> io::Result<()> {
    device.read_adapters()?;

    if let Some(adapter) = device.adapter_mut(adapter) {
        if args.counters {
        } else {
            if args.path {
                adapter.read_paths()?;
            } else if args.counters {
                adapter.read_counters()?;
            }

            for reg in &args.regs {
                match util::parse_hex::<u16>(reg) {
                    Some(offset) => {
                        let reg = if args.path {
                            adapter.path_register_by_offset_mut(offset)
                        } else if args.counters {
                            adapter.counter_register_by_offset_mut(offset)
                        } else {
                            adapter.register_by_offset_mut(offset)
                        };

                        if let Some(reg) = reg {
                            dump_value(reg.value(), args);
                        } else {
                            eprintln!("Warning: invalid offset {}!", offset);
                        }
                    }

                    None => {
                        if args.path || args.counters {
                            eprintln!("Warning: path and counters registers do not have names!");
                        } else {
                            let name: Vec<_> = reg.split('.').collect();

                            if let Some(reg) = adapter.register_by_name_mut(name[0]) {
                                if name.len() > 1 {
                                    if reg.has_field(name[1]) {
                                        dump_value(reg.field(name[1]), args);
                                    } else {
                                        eprintln!("Warning: field name {} not found!", &name[1]);
                                    }
                                } else {
                                    dump_value(reg.value(), args);
                                }
                            } else {
                                eprintln!("Warning: register name {} not found!", &name[0]);
                            }
                        }
                    }
                }
            }
        }
    } else {
        eprintln!("Error: adapter {} not found!", adapter);
        process::exit(1);
    }

    Ok(())
}

fn read(args: &Args) -> io::Result<()> {
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

    if !args.regs.is_empty() {
        device.read_registers()?;

        if let Some(adapter) = args.adapter {
            read_adapter(&mut device, adapter, args)?
        } else {
            read_router(&mut device, args)?;
        }
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

    if args.regs.is_empty() {
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

    if let Err(err) = read(&args) {
        eprintln!("Error: {}", err);
        if err.kind() == ErrorKind::Unsupported {
            eprintln!("Device does not support register access");
        }
        process::exit(1);
    }
}
