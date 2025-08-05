// Monitor events in the Thunderbolt/USB4 domain.
//
// Copyright (C) 2025, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::{
    Colour::{Green, Red, White, Yellow},
    Style,
};
use clap::Parser;
use nix::unistd::Uid;
use regex::Regex;
use std::{
    cell::LazyCell,
    io::{self, IsTerminal},
    process,
};

use tbtools::{
    Device, Kind, debugfs,
    monitor::{Builder, ChangeEvent, Event, TunnelEvent},
    util,
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Monitor events in the Thunderbolt/USB4 domain", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long)]
    domain: Option<u32>,
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
        _ => panic!(),
    }
}

fn color_action(event: &Event) -> String {
    let (action, color) = match event {
        Event::Add(_) => ("add", Green),
        Event::Remove(_) => ("remove", Red),
        Event::Change(..) => ("change", White),
    };

    let action = format!("{action:<6}");

    if io::stdout().is_terminal() {
        color.paint(action).to_string()
    } else {
        action
    }
}

fn color_type_name(type_name: &str) -> String {
    let type_name = match type_name {
        "USB3" => "USB 3",
        "DP" => "DisplayPort",
        "PCI" => "PCIe",
        t => t,
    };
    if io::stdout().is_terminal() {
        Yellow.bold().paint(type_name).to_string()
    } else {
        String::from(type_name)
    }
}

fn dump_change_event(change_event: &ChangeEvent) {
    let re: LazyCell<Regex> =
        LazyCell::new(|| Regex::new(r"^(?:(\d+):(\d+) <-> (\d+):(\d+)\s*)?\((.+)\)").unwrap());
    let bold: Option<Style> = if io::stdout().is_terminal() {
        Some(Style::new().bold())
    } else {
        None
    };

    print!(": ");
    match change_event {
        ChangeEvent::Router { authorized } => print!("authorized ({authorized})"),
        ChangeEvent::Tunnel { event, details } => {
            let (event, color) = match event {
                TunnelEvent::Activated => ("activated", None),
                TunnelEvent::Changed => ("changed", None),
                TunnelEvent::Deactivated => ("deactivated", None),
                TunnelEvent::LowBandwidth => ("low bandwidth", Some(Yellow)),
                TunnelEvent::NoBandwidth => ("no bandwidth", Some(Red)),
            };
            if io::stdout().is_terminal()
                && let Some(color) = color
            {
                print!("{}", color.bold().paint(event));
            } else {
                print!("{event}");
            }
            if let Some(details) = details {
                let Some(caps) = re.captures(details) else {
                    eprintln!("Warning: failed to parse details {details}");
                    return;
                };
                if caps.len() > 2 {
                    let src_route = format!("{:x}", util::parse_hex::<u64>(&caps[1]).unwrap());
                    let src_adapter = format!("{}", caps[2].parse::<u32>().unwrap());
                    let dst_route = format!("{:x}", util::parse_hex::<u64>(&caps[3]).unwrap());
                    let dst_adapter = format!("{}", caps[4].parse::<u32>().unwrap());
                    print!(
                        " (Route {} Adapter {} ⇔  Route {} Adapter {}: {})",
                        bold.map_or(src_route.clone(), |b| b.paint(src_route).to_string()),
                        bold.map_or(src_adapter.clone(), |b| b.paint(src_adapter).to_string()),
                        bold.map_or(dst_route.clone(), |b| b.paint(dst_route).to_string()),
                        bold.map_or(dst_adapter.clone(), |b| b.paint(dst_adapter).to_string()),
                        color_type_name(&caps[5]),
                    );
                } else {
                    print!(" ({})", color_type_name(&caps[1]));
                }
            }
        }
    }
}

fn dump_event(event: Event) {
    let timestamp = util::system_current_timestamp();
    let timestamp = format!("{:6}.{:06}", timestamp.tv_sec(), timestamp.tv_usec());
    let action = color_action(&event);

    match event {
        Event::Add(device) => println!("[{timestamp}] [{action}] {}", color_name(&device)),
        Event::Remove(device) => println!("[{timestamp}] [{action}] {}", color_name(&device)),
        Event::Change(device, change_event) => {
            print!("[{timestamp}] [{action}] {}", color_name(&device));
            if let Some(change_event) = change_event {
                dump_change_event(&change_event);
            }
            println!();
        }
    }
}

fn filter_event(event: &Event, args: &Args) -> bool {
    if let Some(domain) = args.domain {
        match *event {
            Event::Add(ref device) if device.domain_index() == domain => true,
            Event::Remove(ref device) if device.domain_index() == domain => true,
            Event::Change(ref device, _) if device.domain_index() == domain => true,
            _ => false,
        }
    } else {
        true
    }
}

fn start_monitor(args: &Args) -> io::Result<()> {
    let mut monitor = Builder::new()?
        .kind(Kind::Domain)?
        .kind(Kind::Router)?
        .kind(Kind::Xdomain)?
        .kind(Kind::Retimer)?
        .build()?;

    loop {
        match monitor.poll(None) {
            Err(_) => {
                // Handle error
                break;
            }
            Ok(res) if res => {
                for event in monitor.iter_mut().filter(|e| filter_event(e, args)) {
                    dump_event(event);
                }
            }
            Ok(_) => (),
        }
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
        eprintln!("Error: failed to mount debugfs: {err}");
        process::exit(1);
    }

    if let Err(err) = start_monitor(&args) {
        eprintln!("Error: failed to start monitor: {err}");
        process::exit(1);
    }
}
