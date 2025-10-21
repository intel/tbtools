// Dump tunnels in the Thunderbolt/USB4 domain.
//
// Copyright (C) 2025, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use ansi_term::{
    Colour::{Cyan, Yellow},
    Style,
};
use clap::Parser;
use nix::unistd::Uid;
use std::{
    io::{self, ErrorKind, IsTerminal},
    process,
};

use tbtools::{
    Device,
    debugfs::{self, BitFields},
    tunnel::{Direction, Hop, Path, Tunnel, Type},
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Dump tunnels in the Thunderbolt/USB4 domain", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Verbose output (use multiple times to get more detailed output)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn discover_tunnels<'a>(device: &'a Device, devices: &'a [Device]) -> io::Result<Vec<Tunnel<'a>>> {
    let mut tunnels = Vec::new();

    if let Some(adapters) = device.adapters() {
        for adapter in adapters {
            if adapter.is_enabled()
                && let Some(mut tuns) = Tunnel::discover(device, adapter, devices)
            {
                tunnels.append(&mut tuns);
            }
        }
    }

    Ok(tunnels)
}

fn color_type_name(type_name: &str) -> String {
    if io::stdout().is_terminal() {
        Yellow.bold().paint(type_name).to_string()
    } else {
        String::from(type_name)
    }
}

fn color_number(num: u32) -> String {
    let s = format!("{num}");
    if io::stdout().is_terminal() {
        Cyan.paint(s).to_string()
    } else {
        s
    }
}

fn dump_hop(index: usize, hop: &Hop, args: &Args) {
    if args.verbose <= 1 {
        return;
    }

    let bold: Option<Style> = if io::stdout().is_terminal() {
        Some(Style::new().bold())
    } else {
        None
    };

    let index = format!("{index}");
    let route = format!("{:x}", hop.device().route());
    let in_adapter = format!("{}", hop.in_adapter().adapter());
    let in_hop = format!("{}", hop.entry().in_hop());
    let out_adapter = format!("{}", hop.out_adapter().adapter());
    let out_hop = format!("{}", hop.entry().out_hop());

    println!(
        "    {}: Route {} Adapter {} In HopID {} ⇒ Adapter {} Out HopID {}",
        bold.map_or(index.clone(), |b| b.paint(index).to_string()),
        bold.map_or(route.clone(), |b| b.paint(route).to_string()),
        bold.map_or(in_adapter.clone(), |b| b.paint(in_adapter).to_string()),
        bold.map_or(in_hop.clone(), |b| b.paint(in_hop).to_string()),
        bold.map_or(out_adapter.clone(), |b| b.paint(out_adapter).to_string()),
        bold.map_or(out_hop.clone(), |b| b.paint(out_hop).to_string())
    );
}

fn dump_path(path: &Path, args: &Args) {
    if args.verbose == 0 {
        return;
    }

    let bold: Option<Style> = if io::stdout().is_terminal() {
        Some(Style::new().bold())
    } else {
        None
    };

    let src_route = format!("{:x}", path.src_device().route());
    let src_adapter = format!("{}", path.src_adapter().adapter());
    let dst_route = format!("{:x}", path.dst_device().route());
    let dst_adapter = format!("{}", path.dst_adapter().adapter());

    println!(
        "  Route {} Adapter {} ⇒  Route {} Adapter {}: {}",
        bold.map_or(src_route.clone(), |b| b.paint(src_route).to_string()),
        bold.map_or(src_adapter.clone(), |b| b.paint(src_adapter).to_string()),
        bold.map_or(dst_route.clone(), |b| b.paint(dst_route).to_string()),
        bold.map_or(dst_adapter.clone(), |b| b.paint(dst_adapter).to_string()),
        color_type_name(path.name())
    );

    for (i, hop) in path.hops().iter().enumerate() {
        dump_hop(i, hop, args);
    }
}

fn usb3_bw_to_mbps(bw: u32, scale: u32) -> u32 {
    let uframes = (bw as u64 * 512u64) << scale;
    let bw: u32 = (uframes * 8000 / 1000000).try_into().unwrap();
    bw
}

fn dump_usb3_tunnel(tunnel: &Tunnel, args: &Args) -> io::Result<()> {
    if args.verbose == 0 {
        return Ok(());
    }

    if !tunnel.src_device().is_host_router() {
        return Ok(());
    }

    let usb3_down = tunnel.src_adapter();
    if let Some(cs2) = usb3_down.register_by_name("ADP_USB3_GX_CS_2")
        && let Some(cs3) = usb3_down.register_by_name("ADP_USB3_GX_CS_3")
    {
        let scale = cs3.field("Scale");
        let allocated_up_bw = color_number(usb3_bw_to_mbps(
            cs2.field("Allocated Upstream Bandwidth"),
            scale,
        ));
        let allocated_down_bw = color_number(usb3_bw_to_mbps(
            cs2.field("Allocated Downstream Bandwidth"),
            scale,
        ));

        println!("  Allocated Upstream Bandwidth: {allocated_up_bw} Mb/s");
        println!("  Allocated Downstream Bandwidth: {allocated_down_bw} Mb/s");
    } else {
        eprintln!(
            "Warning: invalid USB 3 Down adapter: {}",
            usb3_down.adapter()
        );
    }

    Ok(())
}

fn dump_dp_tunnel(tunnel: &Tunnel, args: &Args) -> io::Result<()> {
    if args.verbose == 0 {
        return Ok(());
    }

    let dp_in = tunnel.src_adapter();
    if let Some(reg) = dp_in.register_by_name("ADP_DP_CS_8")
        && reg.flag("DPME")
    {
        if let Some(cs2) = dp_in.register_by_name("ADP_DP_CS_2")
            && let Some(dp_status) = dp_in.register_by_name("DP_STATUS")
        {
            let granularity = match cs2.field("GR") {
                0 => 250,
                1 => 500,
                2 => 1000,
                val => {
                    eprintln!("Warning: unsupported granularity: {val}");
                    0
                }
            };
            let group = color_number(cs2.field("Group_ID"));
            println!("  Group ID: {group}");
            let estimated_bw = color_number(cs2.field("Estimated BW") * granularity);
            println!("  Estimated Bandwidth: {estimated_bw} Mb/s");
            let allocated_bw = color_number(dp_status.field("Allocated BW") * granularity);
            println!("  Allocated Bandwidth: {allocated_bw} Mb/s");
            let requested_bw = color_number(reg.field("Requested BW") * granularity);
            println!("  Requested Bandwidth: {requested_bw} Mb/s");
        } else {
            eprintln!("Warning: failed to read ADP_DP_CS_2");
        };
    } else if let Some(reg) = dp_in.register_by_name("DP_STATUS") {
        let rate = match reg.field("Link Rate") {
            0 => 1620,
            1 => 2700,
            2 => 5400,
            3 => 8100,
            val => {
                eprintln!("Warning: unsupported rate: {val}");
                0
            }
        };
        let rate = color_number(rate);
        let lanes = color_number(reg.field("Lane Count"));
        println!("Rate: {rate} Mb/s * {lanes}");
    } else {
        eprintln!("Warning: invalid DisplayPort adapter: {}", dp_in.adapter());
    }

    Ok(())
}

fn dump_tunnel(tunnel: &Tunnel, args: &Args) -> io::Result<()> {
    let bold: Option<Style> = if io::stdout().is_terminal() {
        Some(Style::new().bold())
    } else {
        None
    };

    let src_domain = tunnel.src_device().domain_index().to_string();
    let src_route = format!("{:x}", tunnel.src_device().route());
    let src_adapter = format!("{}", tunnel.src_adapter().adapter());
    let dst_domain = tunnel.dst_device().domain_index().to_string();
    let dst_route = format!("{:x}", tunnel.dst_device().route());
    let dst_adapter = format!("{}", tunnel.dst_adapter().adapter());

    let arrow = match tunnel.direction() {
        Direction::Downstream if !tunnel.bidirectional() => "⇒",
        Direction::Upstream if !tunnel.bidirectional() => "⇐",
        _ => "⇔ ",
    };

    println!(
        "Domain {} Route {} Adapter {} {arrow} Domain {} Route {} Adapter {}: {}",
        bold.map_or(src_domain.clone(), |b| b.paint(src_domain).to_string()),
        bold.map_or(src_route.clone(), |b| b.paint(src_route).to_string()),
        bold.map_or(src_adapter.clone(), |b| b.paint(src_adapter).to_string()),
        bold.map_or(dst_domain.clone(), |b| b.paint(dst_domain).to_string()),
        bold.map_or(dst_route.clone(), |b| b.paint(dst_route).to_string()),
        bold.map_or(dst_adapter.clone(), |b| b.paint(dst_adapter).to_string()),
        color_type_name(&tunnel.kind().to_string()),
    );

    match tunnel.kind() {
        Type::Usb3 => dump_usb3_tunnel(tunnel, args)?,
        Type::DisplayPort => dump_dp_tunnel(tunnel, args)?,
        _ => (),
    }

    for path in tunnel.paths() {
        dump_path(path, args);
    }

    Ok(())
}

fn dump(args: &Args) -> io::Result<()> {
    // Pull in all routers and XDomains in the domain.
    let mut devices: Vec<_> = tbtools::find_devices(None)?
        .into_iter()
        .filter(|d| d.domain_index() == args.domain as u32)
        .filter(|d| d.is_router() || d.is_xdomain())
        .collect();

    // Read the adapters as well so that we can figure out the starting and ending adapters.
    for device in &mut devices {
        if !device.is_router() {
            continue;
        }
        device.read_adapters()?;
        if let Some(adapters) = device.adapters_mut() {
            for adapter in adapters {
                if adapter.is_valid() {
                    adapter.read_paths()?;
                }
            }
        }
    }

    let mut tunnels = Vec::new();

    for device in &devices {
        if device.is_router() {
            tunnels.append(&mut discover_tunnels(device, &devices)?);
        }
    }

    if tunnels.is_empty() {
        println!("No tunnels found");
        return Ok(());
    }

    for (i, tunnel) in tunnels.iter().enumerate() {
        dump_tunnel(tunnel, args)?;
        if args.verbose > 0 && i < tunnels.len() - 1 {
            println!();
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

    if let Err(err) = dump(&args) {
        eprintln!("Error: {err}");
        if err.kind() == ErrorKind::Unsupported {
            eprintln!("Error: device does not support register access");
        }
        process::exit(1);
    }
}
