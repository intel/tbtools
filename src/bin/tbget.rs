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
use csv::Writer;
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
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..64))]
    adapter: Option<u8>,
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
    /// Output suitable for scripting (only works with --query)
    #[arg(short = 'S', long)]
    script: bool,
    /// Verbose output (only works with --query)
    #[arg(short, long)]
    verbose: bool,
    /// One or more registers to read in format offset or name[.field]
    regs: Vec<String>,
}

fn dump_value(value: u32, args: &Args) {
    if args.binary {
        println!("{value:#b}");
    } else if args.decimal {
        println!("{value}");
    } else {
        println!("{value:#x}");
    }
}

fn query_register(registers: &[Register], args: &Args) -> io::Result<()> {
    struct NameInfo {
        name: String,
        offset: Option<u16>,
        field: Option<String>,
        range: Option<RangeInclusive<u8>>,
    }

    let mut names = Vec::new();

    if args.regs.is_empty() {
        registers.iter().for_each(|r| {
            if let Some(name) = r.name() {
                names.push(NameInfo {
                    name: name.to_string(),
                    offset: Some(r.offset()),
                    field: None,
                    range: None,
                });
            }
        });
    } else {
        for reg in &args.regs {
            let query: Vec<_> = reg.split('.').collect();

            registers.iter().for_each(|r| {
                if let Some(name) = r.name() {
                    if query.len() > 1 {
                        if name.to_lowercase().contains(&query[0].to_lowercase())
                            || query[0].is_empty()
                        {
                            // Special case "." dump all known names.
                            if query[0].is_empty() && query[1].is_empty() {
                                names.push(NameInfo {
                                    name: name.to_string(),
                                    offset: Some(r.offset()),
                                    field: None,
                                    range: None,
                                });
                            }
                            if let Some(fields) = r.fields() {
                                fields.iter().for_each(|f| {
                                    if f.name().to_lowercase().contains(&query[1].to_lowercase()) {
                                        names.push(NameInfo {
                                            name: name.to_string(),
                                            offset: Some(r.offset()),
                                            field: Some(f.name().to_string()),
                                            range: Some(f.range().clone()),
                                        });
                                    }
                                });
                            }
                        }
                    } else if name.to_lowercase().contains(&query[0].to_lowercase()) {
                        names.push(NameInfo {
                            name: name.to_string(),
                            offset: Some(r.offset()),
                            field: None,
                            range: None,
                        });
                    }
                }
            });
        }
    }

    if args.script {
        let mut writer = Writer::from_writer(io::stdout());
        let mut headers = vec!["domain", "route", "adapter", "index", "name", "field"];

        if args.verbose {
            headers.push("offset");
            headers.push("range_start");
            headers.push("range_end");
        }

        writer.write_record(headers)?;

        for name in &names {
            let mut record = Vec::new();

            record.push(format!("{}", args.domain));
            record.push(format!("{:x}", args.route));
            if let Some(adapter) = &args.adapter {
                record.push(format!("{adapter}"));
            } else {
                record.push(String::new());
            }
            // TODO: Add index with retimer support.
            record.push(String::new());
            record.push(name.name.clone());

            if let Some(field) = &name.field {
                record.push(field.clone());
            } else {
                record.push(String::new());
            }

            if args.verbose {
                if let Some(offset) = name.offset {
                    record.push(format!("0x{offset:04x}"))
                } else {
                    record.push(String::new());
                }
                if let Some(range) = &name.range {
                    record.push(format!("{}", range.start()));
                    record.push(format!("{}", range.end()));
                } else {
                    record.push(String::new());
                    record.push(String::new());
                }
            }

            writer.write_record(record)?;
        }
    } else {
        for name in &names {
            print!("{}", name.name);
            if let Some(field) = &name.field {
                print!(".{field}");
            }
            if args.verbose {
                if let Some(offset) = name.offset {
                    print!(" 0x{offset:04x}");
                }
                if let Some(range) = &name.range {
                    print!(" [{:>02}:{:>02}]", range.start(), range.end());
                }
            }
            println!();
        }
    }
    Ok(())
}

fn query_router(device: &Device, args: &Args) -> io::Result<()> {
    if let Some(registers) = device.registers() {
        query_register(registers, args)?;
    }
    Ok(())
}

fn read_router(device: &mut Device, args: &Args) -> io::Result<()> {
    for reg in &args.regs {
        match util::parse_hex::<u16>(reg) {
            Some(offset) => {
                if let Some(reg) = device.register_by_offset_mut(offset) {
                    dump_value(reg.value(), args);
                } else {
                    eprintln!("Warning: invalid offset {offset}!");
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

fn query_adapter(device: &mut Device, adapter: u8, args: &Args) -> io::Result<()> {
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
            query_register(registers, args)?;
        }
    }

    Ok(())
}

fn read_adapter(device: &mut Device, adapter: u8, args: &Args) -> io::Result<()> {
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
                        eprintln!("Warning: invalid offset {offset}!");
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
        eprintln!("Error: adapter {adapter} not found!");
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
        query_router(&device, args)?;
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
        eprintln!("Error: failed to mount debugfs: {err}");
        process::exit(1);
    }

    if let Err(err) = read(&args) {
        eprintln!("Error: {err}");
        if err.kind() == ErrorKind::Unsupported {
            eprintln!("Device does not support register access");
        }
        process::exit(1);
    }
}
