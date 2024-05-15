// Dump Thunderbolt/USB4 router adapter states
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::{
    Colour::{Green, Red, White, Yellow},
    Style,
};
use clap::Parser;
use csv::Writer;
use nix::unistd::Uid;
use std::io::{self, ErrorKind, IsTerminal, Write};
use std::process;

use tbtools::{
    self,
    debugfs::{self, Adapter, BitFields, State, Type},
    util, Address, Device,
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Dump Thunderbolt/USB4 router adapter states", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device
    #[arg(value_parser = util::parse_route, short, long)]
    route: u64,
    /// Select only specific adapters
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..64))]
    adapter: Option<Vec<u16>>,
    /// Output suitable for scripting
    #[arg(short = 'S', long)]
    script: bool,
}

fn dump_adapter_num(adapter_num: u16, mut record: Option<&mut Vec<String>>) {
    if let Some(ref mut record) = record {
        record.push(adapter_num.to_string());
    } else if io::stdout().is_terminal() {
        print!("{}: ", White.bold().paint(format!("{:>2}", adapter_num)));
    } else {
        print!("{:>2}: ", adapter_num);
    }
}

fn dump_adapter_type(adapter: &Adapter, mut record: Option<&mut Vec<String>>) {
    let mut kind: String = if adapter.is_lane0() {
        String::from("Lane 0")
    } else if adapter.is_lane1() {
        String::from("Lane 1")
    } else {
        adapter.kind().to_string()
    };

    if adapter.is_upstream() {
        kind.push_str(" (upstream)");
    }

    if let Some(ref mut record) = record {
        record.push(kind);
    } else {
        print!("{:<30}", kind);
    }
}

fn protocol_state(adapter: &Adapter) -> (&str, Style) {
    match adapter.kind() {
        Type::PcieDown | Type::PcieUp => {
            if let Some(reg) = adapter.register_by_name("ADP_PCIE_CS_0") {
                if let Some(field) = reg.field_by_name("LTSSM") {
                    let v = reg.field_value(field);
                    match field.value_name(v) {
                        Some("L0 state") => return ("L0", Green.normal()),
                        Some("L1 state") => return ("L1", Green.bold()),
                        Some("L2 state") => return ("L2", Green.bold()),
                        Some("Disabled state") => return ("Disabled", Red.normal()),
                        Some("Hot Reset state") => return ("Hot Reset", Red.normal()),
                        Some(state) => return (state.trim_end_matches(" state"), Yellow.normal()),
                        None => (),
                    }
                }
            }
        }

        Type::Usb3Down | Type::Usb3Up => {
            if let Some(reg) = adapter.register_by_name("ADP_USB3_GX_CS_4") {
                if let Some(field) = reg.field_by_name("PLS") {
                    let v = reg.field_value(field);
                    match field.value_name(v) {
                        Some("U0 state") => return ("U0", Green.normal()),
                        Some("U2 state") => return ("U2", Green.bold()),
                        Some("U3 state") => return ("U3", Green.bold()),
                        Some("Disabled state") => return ("Disabled", Red.normal()),
                        Some("Hot Reset state") => return ("Hot Reset", Red.normal()),
                        Some(state) => return (state.trim_end_matches(" state"), Yellow.normal()),
                        None => (),
                    }
                }
            }
        }

        _ => (),
    }

    ("Enabled", Green.normal())
}

fn dump_adapter_state(adapter: &Adapter, mut record: Option<&mut Vec<String>>) {
    let (name, style) = match adapter.state() {
        State::Disabled => ("Disabled", Red.normal()),
        State::Enabled => protocol_state(adapter),
        State::Training => ("Training/Bonding", Yellow.normal()),
        State::Cl0 => ("CL0", Green.normal()),
        State::Cl0sTx => ("CL0s Tx", Green.bold()),
        State::Cl0sRx => ("CL0s Rx", Green.bold()),
        State::Cl1 => ("CL1", Green.bold()),
        State::Cl2 => ("CL2", Green.bold()),
        State::Cld => ("CLd", Red.normal()),
        _ => ("Unknown", White.dimmed()),
    };

    if let Some(ref mut record) = record {
        record.push(name.to_string());
    } else if io::stdout().is_terminal() {
        print!("{}", style.paint(format!("{:<10}", name)));
    } else {
        print!("{:<10}", name);
    }
}

fn dump_other(mut record: Option<&mut Vec<String>>) {
    if let Some(ref mut record) = record {
        record.push(String::from("Not implemented"));
        record.push(String::new());
    } else {
        print!("Not implemented");
    }
}

fn dump_adapter<W: Write>(adapter: &Adapter, mut writer: Option<&mut Writer<W>>) -> io::Result<()> {
    let mut record: Option<Vec<String>> = if writer.is_some() {
        Some(Vec::new())
    } else {
        None
    };

    dump_adapter_num(adapter.adapter(), record.as_mut());

    if adapter.is_lane() || adapter.is_protocol() {
        dump_adapter_type(adapter, record.as_mut());
        dump_adapter_state(adapter, record.as_mut());
    } else {
        dump_other(record.as_mut());
    }

    if let Some(ref mut writer) = writer {
        writer.write_record(record.unwrap())?;
    } else {
        println!();
    }

    Ok(())
}

fn dump_adapters(device: &mut Device, args: &Args) -> io::Result<()> {
    let mut writer = if args.script {
        let mut writer = Writer::from_writer(io::stdout());
        writer.write_record(["adapter", "type", "state"])?;
        Some(writer)
    } else {
        None
    };

    device.read_adapters()?;

    if let Some(adapter_numbers) = &args.adapter {
        for adapter_num in adapter_numbers {
            if let Some(adapter) = device.adapter(*adapter_num) {
                dump_adapter(adapter, writer.as_mut())?;
            } else {
                eprintln!("Warning: non-existing adapter: {}!", *adapter_num);
            }
        }
    } else if let Some(adapters) = device.adapters() {
        for adapter in adapters {
            dump_adapter(adapter, writer.as_mut())?;
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
            eprintln!("Error: no such device found!");
            process::exit(1);
        }
    };

    dump_adapters(&mut device, args)?;

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
            eprintln!("Error: device does not support register access");
        }
        process::exit(1);
    }
}
