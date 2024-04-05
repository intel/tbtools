// Trace the control traffic of the Thunderbolt/USB4 bus.
//
// Copyright (C) 2024, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::Colour::{Cyan, Green, Purple, Red, White, Yellow};
use clap::{self, Parser, Subcommand};
use is_terminal::IsTerminal;
use nix::unistd::Uid;
use std::{io, path::Path, process};
use tbtools::{
    debugfs::{self, BitFields, Name},
    trace, util, Address, ConfigSpace, Device, Kind, Pdf,
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Trace Thunderbolt/USB4 transport layer configuration traffic", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Shows tracing status (enabled/disabled)
    Status,
    /// Enables tracing
    Enable {
        /// Filter by domain number
        #[arg(short, long)]
        domain: Option<u8>,
    },
    /// Disables tracing
    Disable,
    /// Dumps the current tracing buffer
    Dump {
        /// Trace input file if not reading through tracefs
        #[arg(short, long)]
        input: Option<String>,
        /// Verbose output
        #[arg(short, long, action = clap::ArgAction::Count)]
        verbose: u8,
    },
    /// Clears the tracing buffer
    Clear,
}

fn color_function(function: &str) -> String {
    if io::stdout().is_terminal() {
        White.bold().paint(function).to_string()
    } else {
        function.to_string()
    }
}

fn color_dropped(dropped: bool) -> String {
    if !dropped {
        return String::from("");
    }
    if io::stdout().is_terminal() {
        Red.paint("!").to_string()
    } else {
        "!".to_string()
    }
}

fn color_pdf(pdf: &Pdf) -> String {
    if io::stdout().is_terminal() {
        Green.paint(pdf.to_string()).to_string()
    } else {
        pdf.to_string()
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
        short_name.to_string()
    }
}

fn color_address(address: u16) -> String {
    if io::stdout().is_terminal() {
        Purple.bold().paint(format!("{:04x}", address)).to_string()
    } else {
        format!("{:04x}", address)
    }
}

fn color_tracing(enabled: bool) -> String {
    let s = if enabled { "Enabled" } else { "Disabled" };

    if io::stdout().is_terminal() {
        let c = if enabled { Green } else { White };
        c.paint(s).to_string()
    } else {
        s.to_string()
    }
}

fn dump_header(entry: &trace::Entry, packet: &trace::ControlPacket, device: Option<&Device>) {
    print!(
        "[{:5}.{:06}] ",
        entry.timestamp().tv_sec(),
        entry.timestamp().tv_usec()
    );
    print!("{} ", color_function(entry.function()));
    print!("{}", color_dropped(entry.dropped()));
    print!("{} ", color_pdf(&entry.pdf()));
    print!("Domain {} ", entry.domain_index());
    print!("Route {:x} ", entry.route());

    if let Some(adapter_num) = packet.adapter_num() {
        print!("Adapter {} ", adapter_num);
        if let Some(device) = device {
            if let Some(adapter) = device.adapter(adapter_num) {
                print!("/ {}", adapter.kind());
            }
        }
    }
}

fn dump_name(verbose: u8, name: &dyn Name) {
    if verbose < 1 {
        println!();
        return;
    }
    if let Some(name) = name.name() {
        println!("{}", name);
    } else {
        println!();
    }
}

fn dump_fields(verbose: u8, bitfields: &dyn BitFields<u32>) {
    if verbose < 2 {
        return;
    }
    if let Some(fields) = bitfields.fields() {
        for field in fields {
            let v = bitfields.field(field.name());
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
                "{:15}  [{:>02}:{:>02}] {} {}{}{}",
                "",
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
fn extract_register_info(
    entry: &trace::Entry,
    device: Option<&Device>,
    data_address: u16,
    data: u32,
) -> Option<impl BitFields<u32> + Name> {
    if let Some(device) = device {
        // Use the register metadata to print the details if it is available.
        if let Some(register) = match entry.cs() {
            Some(ConfigSpace::Adapter) => {
                if let Some(adapter_num) = entry.adapter_num() {
                    if let Some(adapter) = device.adapter(adapter_num) {
                        adapter.register_by_offset(data_address)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Some(ConfigSpace::Path) => {
                if let Some(adapter_num) = entry.adapter_num() {
                    if let Some(adapter) = device.adapter(adapter_num) {
                        adapter.path_register_by_offset(data_address)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Some(ConfigSpace::Router) => device.register_by_offset(data_address),
            _ => None,
        } {
            // Clone it so that we can fill in the value and use the field metadata too
            // without changing the actual contents.
            let mut register = register.clone();
            register.set_value(data);

            return Some(register);
        }
    }

    None
}

fn dump_packet(
    entry: &trace::Entry,
    packet: &trace::ControlPacket,
    verbose: u8,
    device: Option<&Device>,
) {
    let mut data_address = packet.data_address().unwrap_or(0);
    let data_start = packet.data_start().unwrap_or(0);

    for (i, f) in packet
        .fields()
        .iter()
        .enumerate()
        .map(|(i, f)| (i as u16, f))
    {
        print!("{:15}", "");

        print!("0x{:02x}", i);
        if verbose > 1 {
            if packet.data().is_some() && i >= data_start {
                print!("/{}", color_address(data_address));
            } else {
                print!("/----");
            }
        }

        let d = f.value();

        print!(" 0x{:08x} ", d);

        print!("0b{:08b}", (d >> 24) & 0xff);
        print!(" {:08b}", (d >> 16) & 0xff);
        print!(" {:08b}", (d >> 8) & 0xff);
        print!(" {:08b} ", d & 0xff);

        print!("{} ", util::bytes_to_ascii(&d.to_be_bytes()));

        if verbose > 0 {
            if packet.data().is_some() && i >= data_start {
                data_address += 1;

                if let Some(register) = extract_register_info(entry, device, data_address - 1, d) {
                    dump_name(verbose, &register);
                    dump_fields(verbose, &register);
                    continue;
                }
            }

            dump_name(verbose, f);
            dump_fields(verbose, f);
        } else {
            println!();
        }
    }
}

fn dump(input: Option<String>, verbose: u8) -> io::Result<()> {
    let mut devices: Vec<Device> = Vec::new();
    let trace_buf;

    if let Some(input) = input {
        trace_buf = trace::buffer(Path::new(&input))?;
    } else {
        trace_buf = trace::live_buffer()?;

        // Only add register information if we are running on a live system.
        if verbose > 0 {
            let devs = tbtools::find_devices(None)?;
            let mut devs: Vec<_> = devs
                .into_iter()
                .filter(|d| d.kind() == Kind::Router)
                .collect();
            devices.append(&mut devs);
        }
    };

    for entry in trace_buf {
        let mut device = devices
            .iter_mut()
            .find(|d| d.domain_index() == entry.domain_index() && d.route() == entry.route());
        if let Some(ref mut device) = device {
            device.read_registers_cached()?;
            device.read_adapters_cached()?;
            if let Some(adapters) = device.adapters_mut() {
                for adapter in adapters {
                    let _ = adapter.read_paths_cached();
                }
            }
        }

        if let Some(packet) = entry.packet() {
            // The kernel records both the event and the immediate receive packet which is the same so
            // we skip the event to avoid outputting duplicate data.
            if packet.is_xdomain() && entry.function() == "tb_event" {
                continue;
            }
            dump_header(&entry, &packet, device.as_deref());
            println!();
            dump_packet(&entry, &packet, verbose, device.as_deref());
        }
    }

    Ok(())
}

fn check_access() {
    if !Uid::current().is_root() {
        eprintln!("Error: debugfs access requires root permissions!");
        process::exit(1);
    }

    if let Err(err) = debugfs::mount() {
        eprintln!("Error: failed to mount debugfs: {}", err);
        process::exit(1);
    }

    if !trace::supported() {
        eprintln!("Error: no tracing support detected");
        process::exit(1);
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    match args.command {
        Commands::Status => {
            check_access();
            if !trace::enabled() {
                println!("Thunderbolt/USB4 tracing: {}", color_tracing(false));
                process::exit(1);
            }
            println!("Thunderbolt/USB4 tracing: {}", color_tracing(true));
        }

        Commands::Enable { domain } => {
            check_access();
            if let Some(domain) = domain {
                trace::add_filter(&Address::Domain { domain })?;
            }
            trace::enable()?;
            println!("Thunderbolt/USB4 tracing: {}", color_tracing(true));
        }

        Commands::Disable => {
            check_access();
            trace::disable()?;
            println!("Thunderbolt/USB4 tracing: {}", color_tracing(false));
        }

        Commands::Dump { input, verbose } => {
            if input.is_none() {
                check_access();
            }

            if verbose > 0 {
                if trace::enabled() {
                    eprintln!(
                        "Note you should disable tracing to avoid this tool affecting the results"
                    );
                }
                if input.is_some() {
                    eprintln!("Note register details need live system");
                }
            }

            dump(input, verbose)?;
        }

        Commands::Clear => {
            check_access();
            trace::clear()?;
            println!("Thunderbolt/USB4 tracing: cleared");
        }
    }

    Ok(())
}
