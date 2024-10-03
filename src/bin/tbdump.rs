// Dump Thunderbolt/USB4 config spaces
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::Colour::{Cyan, Yellow};
use clap::Parser;
use nix::unistd::Uid;
use std::{
    io::{self, ErrorKind, IsTerminal},
    process,
};

use tbtools::{
    self,
    debugfs::{self, BitFields, Name, Register},
    usb4, util, Address, Device,
};

#[derive(Parser, Debug)]
#[command(about = "Dump Thunderbolt/USB4 config spaces", long_about = None)]
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
    /// Verbose output (use multiple times to get more detailed output)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Dump starting from specific capability only (OFFSET and NREGS are relative)
    #[arg(short = 'C', long, group = "cap")]
    cap_id: Option<u16>,
    /// Dump starting from specific VSEC capability only (OFFSET and NREGS are relative)
    #[arg(short = 'V', long, group = "cap")]
    vs_cap_id: Option<u16>,
    /// Number of double words to read
    #[arg(short = 'N', long)]
    nregs: Option<usize>,
    /// Double word offset or name of a register
    offset: Option<String>,
}

fn offset(regs: &[Register], args: &Args) -> u16 {
    if let Some(offset) = &args.offset {
        match util::parse_number(offset) {
            Some(offset) => offset,
            None => {
                // It is name, so lookup for a register with matching name and return its offset.
                // Otherwise bail out, no such register found.
                regs.iter()
                    .find(|r| {
                        r.name()
                            .is_some_and(|n| n.to_lowercase() == offset.to_lowercase())
                    })
                    .map(|r| r.offset())
                    .unwrap_or_else(|| {
                        eprintln!("Error: Valid register name expected!");
                        process::exit(1);
                    })
            }
        }
    } else {
        0
    }
}

fn dump_value(reg: &Register, args: &Args) {
    let value = reg.value();
    print!("0x{:08x}", value);

    if args.verbose > 1 {
        print!(" 0b{:08b}", (value >> 24) & 0xff);
        print!(" {:08b}", (value >> 16) & 0xff);
        print!(" {:08b}", (value >> 8) & 0xff);
        print!(" {:08b} ", value & 0xff);

        print!("{}", util::bytes_to_ascii(&value.to_be_bytes()));
    }
}

fn color_field_value(value: &str) -> String {
    if io::stdout().is_terminal() {
        Cyan.paint(format!("{:>10}", value)).to_string()
    } else {
        format!("{:>10}", value)
    }
}

fn color_field_value_name(name: &str) -> String {
    if io::stdout().is_terminal() {
        Cyan.bold().paint(name).to_string()
    } else {
        name.to_string()
    }
}

fn color_field_short_name(short_name: &str) -> String {
    if io::stdout().is_terminal() {
        Yellow.bold().paint(short_name).to_string()
    } else {
        String::from(short_name)
    }
}

fn dump_regs(regs: &Vec<Register>, args: &Args) {
    let offset = offset(regs, args);
    let mut i = 0;

    for reg in regs {
        if let Some(cap_id) = args.cap_id {
            if reg.cap_id() != cap_id {
                continue;
            }

            // Offset and nregs are now relative to the cap_id.
            if reg.relative_offset() < offset {
                continue;
            }
        } else if let Some(vs_cap_id) = args.vs_cap_id {
            if reg.cap_id() != usb4::CAP_ID_VSEC || vs_cap_id != reg.vs_cap_id() {
                continue;
            }

            // Offset and nregs are now relative to the vs_cap_id.
            if reg.relative_offset() < offset {
                continue;
            }
        } else if reg.offset() < offset {
            continue;
        }

        if let Some(nregs) = args.nregs {
            if i >= nregs {
                return;
            }
            i += 1;
        }

        if args.verbose > 0 {
            print!("0x{:04x} ", reg.offset());
        }

        dump_value(reg, args);

        if args.verbose > 0 {
            if let Some(name) = reg.name() {
                print!(" {:<15}", name);
            }
        }

        println!();

        if args.verbose > 1 {
            if let Some(fields) = reg.fields() {
                for field in fields {
                    let v = reg.field(field.name());
                    let value = color_field_value(&format!("{:#x}", v));
                    let value_name = if let Some(value_name) = field.value_name(v) {
                        format!(" → {}", color_field_value_name(value_name))
                    } else {
                        String::from("")
                    };
                    let short_name = if let Some(short_name) = field.short_name() {
                        format!(" ({})", color_field_short_name(short_name))
                    } else {
                        String::from("")
                    };
                    println!(
                        "  [{:>02}:{:>02}] {} {}{}{}",
                        field.range().start(),
                        field.range().end(),
                        value,
                        field.name(),
                        short_name,
                        value_name,
                    );
                }
            }
        }
    }
}

fn dump_router(device: &mut Device, args: &Args) -> io::Result<()> {
    device.read_registers()?;

    if let Some(regs) = device.registers() {
        dump_regs(regs, args);
    }

    Ok(())
}

fn dump_adapter(device: &mut Device, adapter: u8, args: &Args) -> io::Result<()> {
    device.read_adapters()?;

    if let Some(adapter) = device.adapter_mut(adapter) {
        if args.path {
            adapter.read_paths()?;

            if let Some(regs) = adapter.path_registers() {
                dump_regs(regs, args);
            }
        } else if args.counters {
            adapter.read_counters()?;

            if let Some(regs) = adapter.counter_registers() {
                dump_regs(regs, args);
            }
        } else if let Some(regs) = adapter.registers() {
            dump_regs(regs, args);
        }
    }

    Ok(())
}

fn dump(args: &Args) -> io::Result<()> {
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

    if let Some(adapter) = args.adapter {
        if args.vs_cap_id.is_some() {
            eprintln!("Error: Adapters do not have vendor specific capabilities!");
            process::exit(1);
        }
        dump_adapter(&mut device, adapter, args)?
    } else {
        dump_router(&mut device, args)?
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

    if !Uid::current().is_root() {
        eprintln!("Error: debugfs access requires root permissions");
        process::exit(1);
    }

    if let Err(err) = debugfs::mount() {
        eprintln!("Error: failed to mount debugfs: {}", err);
        process::exit(1);
    }

    if let Err(err) = dump(&args) {
        eprintln!("Error: {}", err);
        if err.kind() == ErrorKind::Unsupported {
            eprintln!("Device does not support register access");
        }
        process::exit(1);
    }
}
