// List Thunderbolt/USB4 devices
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::io;

use ansi_term::Colour::{Cyan, Green, Yellow};
use ansi_term::Style;
use clap::Parser;
use is_terminal::IsTerminal;

use tbtools::{self, Device, Kind, SecurityLevel};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "List Thunderbolt/USB4 devices", long_about = None)]
struct Args {
    /// List all devices, not just routers
    #[arg(short = 'A', long)]
    all: bool,
    /// Output suitable for scripting
    #[arg(short = 'S', long, group = "output")]
    script: bool,
    /// List devices in tree format
    #[arg(short, long, group = "output")]
    tree: bool,
    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn indent(args: &Args, device: &Device) -> String {
    if args.tree {
        if args.all {
            return " ".repeat(((device.depth() + 1) * 4) as usize);
        }
        return " ".repeat((device.depth() * 4) as usize);
    }
    String::from("")
}

fn color_root() -> String {
    if io::stdout().is_terminal() {
        return Yellow.paint("/").to_string();
    }

    String::from("/")
}

fn kind(device: &Device) -> String {
    match device.kind() {
        Kind::Domain => String::from("Domain"),
        Kind::Router => String::from("Router"),
        Kind::Retimer => String::from("Retimer"),
        Kind::Xdomain => String::from("XDomain"),
        _ => String::from("Unknown"),
    }
}

fn color_kind(device: &Device) -> String {
    let name = kind(device);

    if io::stdout().is_terminal() {
        return Cyan.paint(name).to_string();
    }

    name
}

fn color_name(device: &Device) -> String {
    let bold: Option<Style> = if io::stdout().is_terminal() {
        Some(Style::new().bold())
    } else {
        None
    };
    let domain = device.domain_index().to_string();
    let route = format!("{:x}", device.route());

    match device.kind() {
        Kind::Domain => {
            format!(
                "Domain {}",
                bold.map_or(domain.clone(), |b| b.paint(domain).to_string())
            )
        }

        Kind::Router | Kind::Xdomain => {
            format!(
                "Domain {} Route {}",
                bold.map_or(domain.clone(), |b| b.paint(domain).to_string()),
                bold.map_or(route.clone(), |b| b.paint(route).to_string())
            )
        }

        Kind::Retimer => {
            let route = format!("{:x}", device.route());
            let adapter_num = device.adapter_num().to_string();
            let index = device.index().to_string();

            format!(
                "Domain {} Route {} Adapter {} Index {}",
                bold.map_or(domain.clone(), |b| b.paint(domain).to_string()),
                bold.map_or(route.clone(), |b| b.paint(route).to_string()),
                bold.map_or(adapter_num.clone(), |b| b.paint(adapter_num).to_string()),
                bold.map_or(index.clone(), |b| b.paint(index).to_string())
            )
        }

        _ => todo!(),
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

fn print_domain(args: &Args, tb: &Device) {
    if args.script {
        println!("{},,,,,,,,{}", tb.domain_index(), kind(tb));
    } else {
        let mut indent = String::from("");

        if args.tree {
            println!("{}:  {}", color_root(), color_name(tb));
            indent = String::from("    ");
        } else {
            println!("{}", color_name(tb));
        }

        if args.verbose {
            println!("{}  Type: {}", indent, color_kind(tb));

            let security_level = match tb.security_level() {
                Some(SecurityLevel::None) => "None",
                Some(SecurityLevel::User) => "User",
                Some(SecurityLevel::Secure) => "Secure Connect",
                Some(SecurityLevel::DpOnly) => "DisplayPort Only",
                Some(SecurityLevel::UsbOnly) => "USB and DisplayPort Only",
                Some(SecurityLevel::NoPcie) => "PCIe tunneling disabled",
                _ => "Unknown",
            };
            println!("{}  Security Level: {}", indent, security_level);
            println!(
                "{}  Deauthorization: {}",
                indent,
                color_bool(tb.deauthorization().unwrap_or(false))
            );
            println!(
                "{}  IOMMU DMA protection: {}",
                indent,
                color_bool(tb.iommu_dma_protection().unwrap_or(false))
            );
        }
    }
}

fn print_router(args: &Args, sw: &Device) {
    if args.script {
        print!(
            "{},{:x},,,{:04x},{:04x}",
            sw.domain_index(),
            sw.route(),
            sw.vendor(),
            sw.device()
        );
        if let Some(vendor_name) = sw.vendor_name() {
            print!(",{}", vendor_name);
        }
        if let Some(device_name) = sw.device_name() {
            print!(",{}", device_name);
        }

        println!(",{}", kind(sw));
    } else {
        let indent = indent(args, sw);

        if args.tree {
            print!("{}", indent);
        }

        print!(
            "{}: {:04x}:{:04x}",
            color_name(sw),
            sw.vendor(),
            sw.device()
        );

        if let Some(vendor_name) = sw.vendor_name() {
            print!(" {}", vendor_name);
        }
        if let Some(device_name) = sw.device_name() {
            print!(" {}", device_name);
        }

        println!();

        if args.verbose {
            println!("{}  Type: {}", indent, color_kind(sw));

            if sw.is_device_router() || sw.is_xdomain() {
                print!("{}  Speed (Rx/Tx): ", indent);

                if let Some(rx_speed) = sw.rx_speed() {
                    if let Some(rx_lanes) = sw.rx_lanes() {
                        print!("{}", rx_speed * rx_lanes);
                    }
                }

                print!("/");

                if let Some(tx_speed) = sw.tx_speed() {
                    if let Some(tx_lanes) = sw.tx_lanes() {
                        print!("{}", tx_speed * tx_lanes);
                    }
                }

                println!(" Gb/s");
            }

            if sw.is_device_router() {
                println!(
                    "{}  Authorized: {}",
                    indent,
                    color_bool(sw.authorized().unwrap_or(false))
                );
            }

            if let Some(unique_id) = sw.unique_id() {
                println!("{}  UUID: {}", indent, unique_id);
            }

            if let Some(generation) = sw.generation() {
                print!("{}  generation: ", indent);
                match generation {
                    1..=3 => println!("Thunderbolt {}", generation),
                    4 => println!("USB4"),
                    _ => println!("Unknown"),
                }
            }

            if let Some(version) = sw.nvm_version() {
                println!(
                    "{}  NVM version: {:x}.{:x}",
                    indent, version.major, version.minor
                );
            }
        }
    }
}

fn print_retimer(args: &Args, rt: &Device) {
    if args.script {
        println!(
            "{},{:x},{},{},{:04x},{:04x},,,{}",
            rt.domain_index(),
            rt.route(),
            rt.adapter_num(),
            rt.index(),
            rt.vendor(),
            rt.device(),
            kind(rt),
        );
    } else {
        let indent = indent(args, rt);

        if args.tree {
            print!("{}", indent);
        }
        println!(
            "{}: {:04x}:{:04x}",
            color_name(rt),
            rt.vendor(),
            rt.device()
        );

        if args.verbose {
            println!("{}  Type: {}", indent, color_kind(rt));

            if let Some(version) = rt.nvm_version() {
                println!(
                    "{}  NVM version: {:x}.{:x}",
                    indent, version.major, version.minor
                );
            }
        }
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let devices = tbtools::find_devices(None)?;
    let devices: Vec<_> = devices
        .iter()
        .filter(|d| match d.kind() {
            Kind::Router => true,
            Kind::Retimer | Kind::Domain | Kind::Xdomain => args.all,
            _ => false,
        })
        .collect();

    if !args.script && devices.is_empty() {
        println!("No Thunderbolt/USB4 devices found");
        return Ok(());
    }

    if args.script {
        println!("domain,route,adapter,index,vendor,device,vendor_name,device_name,type");
    }

    for (i, device) in devices.iter().enumerate() {
        match device.kind() {
            Kind::Domain => print_domain(&args, device),
            Kind::Xdomain => print_router(&args, device),
            Kind::Router => print_router(&args, device),
            Kind::Retimer => print_retimer(&args, device),
            _ => (),
        }

        if !args.script && args.verbose && i < devices.len() - 1 {
            println!();
        }
    }

    Ok(())
}
