// Authorize Thunderbolt/USB4 devices
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::fmt::{self, Write};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write as IoWrite};
use std::process;

use clap::Parser;
use rand::prelude::*;

use tbtools::{Address, Device, Kind, SecurityLevel, util};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Authorize Thunderbolt/USB4 devices", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device to authorize
    #[arg(value_parser = util::parse_route, short, long)]
    route: u64,
    /// De-authorize instead of authorize
    #[arg(short = 'D', long)]
    deauthorize: bool,
    /// Authorize with key (generated does not exist)
    #[arg(short = 'A', long, group = "key")]
    add_key_path: Option<String>,
    /// Challenge with key
    #[arg(short = 'C', long, group = "key")]
    challenge_key_path: Option<String>,
}

fn is_deauthorization_supported(device: &Device) -> bool {
    device.domain().unwrap().deauthorization().unwrap_or(false)
}

fn is_secure_supported(device: &Device) -> bool {
    if device.domain().unwrap().security_level().unwrap() != SecurityLevel::Secure {
        return false;
    }
    device.has_key()
}

fn gen_key() -> Result<String, fmt::Error> {
    let mut data = [0u8; 32];

    rand::thread_rng().fill_bytes(&mut data);

    let mut key = String::new();
    for b in data {
        write!(key, "{b:02x}")?;
    }

    Ok(key)
}

fn main() -> io::Result<()> {
    let args = Args::parse();

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

    if device.kind() != Kind::Router {
        eprintln!("Error: Only routers can be authorized!");
        process::exit(1);
    }

    if args.deauthorize {
        if !is_deauthorization_supported(&device) {
            eprintln!("Error: Domain does not support de-authorization!");
            process::exit(1);
        }

        return tbtools::authorize_device(&mut device, 0);
    }

    let authorized: u32;

    if let Some(path) = &args.add_key_path {
        if !is_secure_supported(&device) {
            eprintln!("Error: Domain security level is not 'secure'");
            process::exit(1);
        }

        let key = match gen_key() {
            Err(err) => {
                eprintln!("Error: Key generation failed {err}");
                process::exit(1);
            }
            Ok(key) => key,
        };

        device.set_key(&key)?;

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path);
        write!(file?, "{key}")?;

        authorized = 1;
    } else if let Some(path) = &args.challenge_key_path {
        if !is_secure_supported(&device) {
            eprintln!("Error: Domain security level is not 'secure'");
            process::exit(1);
        }

        let mut file = File::open(path)?;
        let mut key = String::new();
        file.read_to_string(&mut key)?;

        device.set_key(&key)?;
        authorized = 2;
    } else {
        authorized = 1;
    }

    if let Err(err) = tbtools::authorize_device(&mut device, authorized) {
        eprintln!(
            "Error: Device {} authorization failed {}",
            device.kernel_name(),
            err
        );
        process::exit(1);
    }

    Ok(())
}
