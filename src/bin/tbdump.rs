// Dump Thunderbolt/USB4 config spaces
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::Colour::{Cyan, Green, Red, White, Yellow};
use clap::Parser;
use nix::unistd::Uid;
use std::{
    io::{self, ErrorKind, IsTerminal},
    process,
};

use tbtools::{
    self, Address, Device, Version,
    debugfs::{self, BitFields, Name, Register},
    drom::{Drom, DromEntry, RankType, SingleDataPathPreference, TmuMode, TmuRate},
    usb4, util,
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
    /// Dump router DROM instead of registers
    #[arg(short = 'R', long)]
    drom: bool,
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
    print!("0x{value:08x}");

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
        Cyan.paint(format!("{value:>10}")).to_string()
    } else {
        format!("{value:>10}")
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

fn color_goodbad(yesno: bool) -> String {
    if io::stdout().is_terminal() {
        if yesno {
            Green.bold().paint("✔").to_string()
        } else {
            Red.bold().paint("✘").to_string()
        }
    } else if yesno {
        String::from("✔")
    } else {
        String::from("✘")
    }
}

fn color_bool(val: bool) -> String {
    (match val {
        true => {
            if io::stdout().is_terminal() {
                Green.paint("Yes").to_string()
            } else {
                "Yes".to_string()
            }
        }
        false => String::from("No"),
    })
    .to_string()
}

fn color_value(value: &str) -> String {
    if io::stdout().is_terminal() {
        Cyan.paint(value.to_string()).to_string()
    } else {
        value.to_string()
    }
}

fn color_adapter_num(adapter_num: u8) -> String {
    if io::stdout().is_terminal() {
        White.bold().paint(format!("{adapter_num}")).to_string()
    } else {
        format!("{adapter_num}").to_string()
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

        if args.verbose > 0
            && let Some(name) = reg.name()
        {
            print!(" {name:<15}");
        }

        println!();

        if args.verbose > 1
            && let Some(fields) = reg.fields()
        {
            for field in fields {
                let v = reg.field(field.name());
                let value = color_field_value(&format!("{v:#x}"));
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

fn dump_bytes(bytes: &[u8], offset: usize, args: &Args) {
    let mut offset = offset;

    for chunks in bytes.chunks(8) {
        if args.verbose > 0 {
            print!("0x{offset:04x} ");
        }

        for byte in chunks {
            print!("0x{byte:02x} ");
        }

        if args.verbose > 0 {
            let indent = 8 * (4 + 1) - chunks.len() * 4;
            print!("{:>1$}", util::bytes_to_ascii(chunks), indent);
        }

        println!();
        offset += chunks.len();
    }
}

fn dump_drom(drom: &Drom, args: &Args) {
    if args.verbose < 2 {
        dump_bytes(drom.bytes(), 0, args);
        return;
    }

    dump_bytes(drom.header(), 0, args);

    if drom.is_tb3_compatible() {
        println!(
            "  CRC8: {} {}",
            color_value(&format!("{:#x}", drom.crc8().unwrap())),
            color_goodbad(drom.is_crc8_valid()),
        );
        println!(
            "  UUID: {}",
            color_value(&format!("{:#x}", drom.uuid().unwrap()))
        );
    }

    println!(
        "  CRC32: {} {}",
        color_value(&format!("{:#x}", drom.crc32())),
        color_goodbad(drom.is_crc32_valid()),
    );
    println!(
        "  Version: {} ({})",
        color_value(&format!("{}", drom.version())),
        if drom.is_tb3_compatible() {
            "Thunderbolt 3 Compatible"
        } else {
            "USB4"
        }
    );

    let mut entries = drom.entries();

    while let Some(entry) = entries.next() {
        dump_bytes(entries.bytes(), entries.start(), args);

        match entry {
            // Adapter entries.
            DromEntry::Adapter {
                disabled,
                adapter_num,
            } => {
                println!("  Adapter {}", color_adapter_num(adapter_num));
                if disabled {
                    println!("    Disabled");
                }
            }

            DromEntry::UnusedAdapter { adapter_num } => {
                println!("  Unused Adapter {}", color_adapter_num(adapter_num))
            }

            DromEntry::DisplayPortAdapter {
                adapter_num,
                preferred_lane_adapter_num,
                preference_valid,
            } => {
                println!("  DisplayPort Adapter {}", color_adapter_num(adapter_num));
                if preference_valid {
                    println!(
                        "    Preferred Lane adapter: {}",
                        color_adapter_num(preferred_lane_adapter_num),
                    );
                }
            }

            DromEntry::LaneAdapter {
                adapter_num,
                lane_1_adapter,
                dual_lane_adapter,
                dual_lane_adapter_num,
            } => {
                println!(
                    "  Lane {} Adapter {}",
                    color_adapter_num(adapter_num),
                    if lane_1_adapter { 1 } else { 0 }
                );
                if dual_lane_adapter {
                    println!(
                        "    Dual Lane Adapter: {}",
                        color_adapter_num(dual_lane_adapter_num)
                    );
                }
            }

            DromEntry::PcieUpAdapter {
                adapter_num,
                function_num,
                device_num,
            } => {
                println!("  PCIe Upstream Adapter {}", color_adapter_num(adapter_num));
                println!(
                    "    {}",
                    color_value(&format!("{device_num:02x}.{function_num:x}"))
                );
            }

            DromEntry::PcieDownAdapter {
                adapter_num,
                function_num,
                device_num,
            } => {
                println!(
                    "  PCIe Downstream Adapter {}",
                    color_adapter_num(adapter_num)
                );
                println!(
                    "    {}",
                    color_value(&format!("{device_num:02x}.{function_num:x}"))
                );
            }

            // Generic entries.
            DromEntry::Generic { kind, .. } => {
                println!("  Generic Entry: {}", color_value(&format!("{kind:#x}")));
            }

            DromEntry::AsciiVendorName(vendor) => println!("  Vendor: {vendor}"),
            DromEntry::AsciiModelName(model) => println!("  Model: {model}"),

            DromEntry::Utf16VendorName(vendor) => println!("  Vendor: {vendor}"),
            DromEntry::Utf16ModelName(model) => println!("  Model: {model}"),

            DromEntry::Tmu { mode, rate } => {
                println!(
                    "  TMU: {}",
                    match mode {
                        TmuMode::Unknown => "Unknown",
                        TmuMode::Off => "Off",
                        TmuMode::Unidirectional => {
                            if rate == TmuRate::HiFi {
                                "Unidirectional, HiFi"
                            } else if rate == TmuRate::LowRes {
                                "Unidirectional, LowRes"
                            } else {
                                "Unidirectional"
                            }
                        }
                        TmuMode::Bidirectional => {
                            if rate == TmuRate::HiFi {
                                "Bidirectional, HiFi"
                            } else {
                                "Bidirectional"
                            }
                        }
                    }
                );
            }

            DromEntry::ProductDescriptor {
                usb4_version:
                    Version {
                        major: usb4_major,
                        minor: usb4_minor,
                    },
                vendor,
                product,
                fw_version:
                    Version {
                        major: fw_major,
                        minor: fw_minor,
                    },
                test_id,
                hw_revision,
            } => {
                println!("  Product Descriptor:");
                println!(
                    "    bcdUSBSpec: {}",
                    color_value(&format!("{usb4_major:x}.{usb4_minor:x}"))
                );
                println!("    idVendor: {}", color_value(&format!("{vendor:04x}")));
                println!("    idProduct: {}", color_value(&format!("{product:04x}")));
                println!(
                    "    bcdProductFWRevision: {}",
                    color_value(&format!("{fw_major:x}.{fw_minor:x}"))
                );
                println!("    TID: {}", color_value(&format!("{test_id:04x}")));
                println!(
                    "    productHWRevision: {}",
                    color_value(&format!("{hw_revision}"))
                );
            }

            DromEntry::SerialNumber {
                lang_id,
                serial_number,
            } => {
                println!("  Serial Number:");
                println!("    wLANGID: {}", color_value(&format!("{lang_id}")));
                println!(
                    "    SerialNumber: {}",
                    color_value(&serial_number.to_string())
                );
            }

            DromEntry::Usb3PortMapping(mappings) => {
                println!("  USB Port Mappings:");
                for mapping in mappings {
                    println!("    USB 3 Port {}", mapping.usb3_port_num);
                    println!("      xHCI Index: {}", mapping.xhci_index);
                    if mapping.usb_type_c {
                        println!("      PD Port: {}", mapping.pd_port_num);
                    }
                    if mapping.tunneling {
                        println!(
                            "      USB 3 Downstream Adapter: {}",
                            color_adapter_num(mapping.usb3_adapter_num)
                        );
                    }
                }
            }

            DromEntry::SingleDataPath(pref) => {
                println!(
                    "  Preferred Single Data Path: {}",
                    match pref {
                        SingleDataPathPreference::PcieTunneling => String::from("PCIe Tunneling"),
                        SingleDataPathPreference::Usb3GenTTunneling =>
                            String::from("USB 3 GenT Tunneling"),
                        SingleDataPathPreference::Reserved(v) => format!("Reserved {v}"),
                    }
                );
            }

            DromEntry::DptxRanking {
                nrr,
                mh,
                preferred_order,
                mu,
                records,
            } => {
                println!("  DPTX Ranking:");
                println!("    NRR: {}", color_value(&format!("{nrr}")));
                println!("    MST Hub: {}", color_bool(mh));
                println!(
                    "    Preferred Order: {}",
                    color_value(&format!("{preferred_order}"))
                );
                if mh {
                    println!("    MST Upstream: {}", color_value(&format!("{mu}")));
                }
                if nrr > 0 {
                    println!("    Records:");
                    for record in records {
                        println!(
                            "      Rank Type: {}",
                            match record.rank_type {
                                RankType::Power => String::from("Power"),
                                RankType::Performance => String::from("Performance"),
                                RankType::Reserved(v) => format!("Reserved {v}"),
                            }
                        );
                        println!("      Rank: {}", color_value(&format!("{}", record.rank)));
                    }
                }
            }

            DromEntry::Unknown(_) => println!("Unknown Entry"),
        }
    }
}

fn dump_router(device: &mut Device, args: &Args) -> io::Result<()> {
    device.read_registers()?;

    if args.drom {
        device.read_drom()?;

        if let Some(drom) = device.drom() {
            dump_drom(drom, args);
        }
    } else if let Some(regs) = device.registers() {
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
        } else if args.drom {
            eprintln!("Error: Only routers have DROM!");
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

    debugfs::mount().unwrap_or_else(|e| {
        eprintln!("Error: failed to mount debugfs: {e}");
        process::exit(1);
    });

    dump(&args).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        if e.kind() == ErrorKind::Unsupported {
            eprintln!("Device does not support register access");
        }
        process::exit(1);
    });
}
