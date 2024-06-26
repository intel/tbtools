// Trace the control traffic of the Thunderbolt/USB4 bus.
//
// Copyright (C) 2024, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::Colour::{Cyan, Green, Purple, Red, White, Yellow};
use clap::{self, Parser, Subcommand};
use csv::Writer;
use nix::{sys::time::TimeVal, unistd::Uid};
use std::{
    io::{self, IsTerminal, Write},
    path::Path,
    process,
};
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
        /// Output suitable for scripting
        #[arg(short = 'S', long, group = "output")]
        script: bool,
        /// Timestamp as system wall clock time instead of seconds from boot
        #[arg(short = 'T', long)]
        time: bool,
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

fn timestamp(ts: &TimeVal, boot_time: Option<TimeVal>) -> TimeVal {
    if let Some(boot_time) = boot_time {
        boot_time + *ts
    } else {
        *ts
    }
}

fn dump_header(
    entry: &trace::Entry,
    packet: &trace::ControlPacket,
    record: Option<&mut Vec<String>>,
    device: Option<&Device>,
    boot_time: Option<TimeVal>,
) {
    let ts = timestamp(entry.timestamp(), boot_time);

    if let Some(record) = record {
        record.push(format!(
            "{}.{:06}",
            entry.timestamp().tv_sec(),
            entry.timestamp().tv_usec()
        ));
        if ts != *entry.timestamp() {
            record.push(format!("{}.{:06}", ts.tv_sec(), ts.tv_usec()));
        } else {
            record.push(String::new());
        }
        record.push(entry.function().to_string());
        record.push(entry.dropped().to_string());
        record.push(entry.pdf().to_string());
        if let Some(cs) = entry.cs() {
            record.push(cs.to_string());
        } else {
            record.push(String::new());
        }
        record.push(format!("{}", entry.domain_index()));
        record.push(format!("{:x}", entry.route()));
        if let Some(adapter_num) = packet.adapter_num() {
            record.push(adapter_num.to_string());
            if let Some(device) = device {
                if let Some(adapter) = device.adapter(adapter_num) {
                    record.push(adapter.kind().to_string());
                } else {
                    record.push(String::new());
                }
            } else {
                record.push(String::new());
            }
        } else {
            record.push(String::new());
            record.push(String::new());
        }
    } else {
        print!("[{:5}.{:06}] ", ts.tv_sec(), ts.tv_usec());
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

        println!();
    }
}

fn dump_name(verbose: u8, record: Option<&mut Vec<String>>, name: &dyn Name) {
    if verbose < 1 {
        println!();
        return;
    }
    if let Some(name) = name.name() {
        if let Some(record) = record {
            record.push(name.to_string());
        } else {
            println!("{}", name);
        }
    } else if let Some(record) = record {
        record.push(String::new());
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
                    dump_name(verbose, None, &register);
                    dump_fields(verbose, &register);
                    continue;
                }
            }

            dump_name(verbose, None, f);
            dump_fields(verbose, f);
        } else {
            println!();
        }
    }
}

fn dump_script_packet<W: Write>(
    entry: &trace::Entry,
    packet: &trace::ControlPacket,
    header: &[String],
    writer: &mut Writer<W>,
    verbose: u8,
    device: Option<&Device>,
) -> io::Result<()> {
    let mut data_address = packet.data_address().unwrap_or(0);
    let data_start = packet.data_start().unwrap_or(0);

    for (i, f) in packet
        .fields()
        .iter()
        .enumerate()
        .map(|(i, f)| (i as u16, f))
    {
        let mut record = header.to_owned();

        record.push(format!("0x{:02x}", i));
        if verbose > 1 {
            if packet.data().is_some() && i >= data_start {
                record.push(format!("0x{:04x}", data_address));
            } else {
                record.push(String::new());
            }
        } else {
            record.push(String::new());
        }
        record.push(format!("0x{:08x}", f.value()));

        if verbose > 0 {
            if packet.data().is_some() && i >= data_start {
                data_address += 1;

                if let Some(register) =
                    extract_register_info(entry, device, data_address - 1, f.value())
                {
                    dump_name(verbose, Some(&mut record), &register);
                } else {
                    dump_name(verbose, Some(&mut record), f);
                }
            } else {
                dump_name(verbose, Some(&mut record), f);
            }
        } else {
            record.push(String::new());
        }

        writer.write_record(record)?;
    }

    Ok(())
}

fn dump(input: Option<String>, script: bool, time: bool, verbose: u8) -> io::Result<()> {
    let mut devices: Vec<Device> = Vec::new();
    let trace_buf;

    if let Some(input) = input {
        trace_buf = trace::buffer(Path::new(&input)).unwrap_or_else(|e| {
            eprintln!("Error: failed open trace input file: {}", e);
            process::exit(1);
        });
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

    let boot_time = if time {
        Some(util::system_boot_time()?)
    } else {
        None
    };

    let mut writer = if script {
        let mut writer = Writer::from_writer(io::stdout());
        // Add header.
        writer.write_record([
            "entry",
            "timestamp",
            "datetime",
            "function",
            "dropped",
            "pdf",
            "cs",
            "domain",
            "route",
            "adapter",
            "adapter_type",
            "offset",
            "data_offset",
            "value",
            "name",
        ])?;
        Some(writer)
    } else {
        None
    };

    let mut line = 0;

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

            if let Some(ref mut writer) = writer {
                let mut header: Vec<String> = vec![line.to_string()];
                // Header part is always the same.
                dump_header(
                    &entry,
                    &packet,
                    Some(&mut header),
                    device.as_deref(),
                    boot_time,
                );
                dump_script_packet(&entry, &packet, &header, writer, verbose, device.as_deref())?;
                line += 1;
            } else {
                dump_header(&entry, &packet, None, device.as_deref(), boot_time);
                dump_packet(&entry, &packet, verbose, device.as_deref());
            }
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

        Commands::Dump {
            input,
            script,
            time,
            verbose,
        } => {
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

            if time && input.is_some() {
                eprintln!("Note you should run on the same system you took the trace to get accurate times");
            }

            dump(input, script, time, verbose)?;
        }

        Commands::Clear => {
            check_access();
            trace::clear()?;
            println!("Thunderbolt/USB4 tracing: cleared");
        }
    }

    Ok(())
}
