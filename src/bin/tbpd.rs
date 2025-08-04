// Helper for configuring USB4 ports.
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::process;

use clap::{self, Parser};

use tbtools::{
    Address,
    typec::{self, AltMode, AltModeControl},
    util,
};

/// Control USB4 Type-C port alternate modes
///
/// This command can be used to move the Type-C port associated with the USB4 port
/// into different alternate modes. If the -m is not passed will output the current
/// mode the Type-C port is currently.
#[derive(Parser)]
#[command(version)]
#[command(about, long_about)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device
    #[arg(value_parser = util::parse_route, short, long, default_value_t = 0)]
    route: u64,
    /// Lane 0 adapter number (1 - 64)
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..64))]
    adapter: u16,
    /// Alternate mode the port is put into
    #[arg(short, long, value_enum)]
    mode: Option<AltMode>,
}

fn main() {
    let args = Args::parse();

    let ac = match typec::controller() {
        Ok(ac) => ac,
        Err(err) => {
            eprintln!("Error: failed to open Type-C controller: {err}");
            process::exit(1);
        }
    };

    let address = Address::Adapter {
        domain: args.domain,
        route: args.route,
        adapter: args.adapter as u8,
    };

    if let Some(mode) = args.mode {
        if let Err(err) = ac.enter_mode(&address, &mode) {
            eprintln!("Error: failed to enter {} mode: {}", &mode, err);
            process::exit(1);
        }
        println!(
            "Domain {} Route {:x} Adapter {}: entered {}",
            args.domain, args.route, args.adapter, &mode
        );
    } else {
        let mode = match ac.current_mode(&address) {
            Ok(mode) => mode,
            Err(err) => {
                eprintln!("Error: failed to read current mode: {err}");
                process::exit(1);
            }
        };
        println!(
            "Domain {} Route {:x} Adapter {}: {}",
            args.domain, args.route, args.adapter, mode
        );
    }
}
