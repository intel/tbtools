// Read Thunderbolt/USB4 config spaces
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::{
    io::{self, ErrorKind},
    ops::RangeInclusive,
    process,
};

use clap::Parser;
use nix::unistd::Uid;

use tbtools::{
    debugfs::{self, BitFields, Name, Register},
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
    /// Query all register names starting with name
    #[arg(short = 'Q', long, group = "output")]
    query: bool,
    /// Verbose output (only works with --query)
    #[arg(short, long)]
    verbose: bool,
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

fn query_register(registers: &[Register], args: &Args) {
    struct NameInfo {
        name: String,
        offset: Option<u32>,
        range: Option<RangeInclusive<u8>>,
    }

    let mut names = Vec::new();

    if args.regs.is_empty() {
        registers.iter().for_each(|r| {
            if let Some(name) = r.name() {
                names.push(NameInfo {
                    name: name.to_string(),
                    offset: Some(r.offset()),
                    range: None,
                });
            }
        });
    } else {
        for reg in &args.regs {
            let reg: Vec<_> = reg.split('.').collect();

            registers.iter().for_each(|r| {
                if let Some(name) = r.name() {
                    if reg.len() > 1 {
                        if name.to_lowercase().contains(&reg[0].to_lowercase()) || reg[0].is_empty()
                        {
                            if let Some(fields) = r.fields() {
                                fields.iter().for_each(|f| {
                                    if f.name().to_lowercase().contains(&reg[1].to_lowercase()) {
                                        names.push(NameInfo {
                                            name: format!("{}.{}", name, f.name()),
                                            offset: Some(r.offset()),
                                            range: Some(f.range().clone()),
                                        });
                                    }
                                });
                            }
                        }
                    } else if name.to_lowercase().contains(&reg[0].to_lowercase()) {
                        names.push(NameInfo {
                            name: name.to_string(),
                            offset: Some(r.offset()),
                            range: None,
                        });
                    }
                }
            });
        }
    }

    for name in &names {
        print!("{}", name.name);
        if args.verbose {
            if let Some(offset) = name.offset {
                print!(" 0x{:04x}", offset);
            }
            if let Some(range) = &name.range {
                print!(" [{:>02}:{:>02}]", range.start(), range.end());
            }
        }
        println!();
    }
}

fn query_router(device: &Device, args: &Args) {
    if let Some(registers) = device.registers() {
        query_register(registers, args);
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

fn query_adapter(device: &mut Device, adapter: u16, args: &Args) -> io::Result<()> {
    device.read_adapters()?;

    if let Some(adapter) = device.adapter_mut(adapter) {
        if let Some(registers) = if args.path {
            adapter.read_paths()?;
            adapter.path_registers()
        } else if args.counters {
            adapter.read_counters()?;
            adapter.counter_registers()
        } else {
            adapter.registers()
        } {
            query_register(registers, args);
        }
    }

    Ok(())
}

fn read_adapter(device: &mut Device, adapter: u16, args: &Args) -> io::Result<()> {
    device.read_adapters()?;

    if let Some(adapter) = device.adapter_mut(adapter) {
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

    device.read_registers()?;

    if let Some(adapter) = args.adapter {
        if args.query {
            query_adapter(&mut device, adapter, args)?;
        } else {
            read_adapter(&mut device, adapter, args)?
        }
    } else if args.query {
        query_router(&device, args);
    } else {
        read_router(&mut device, args)?;
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

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
