// Thunderbolt/USB4 live device manager
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use nix::unistd::Uid;
use std::process;
use tbtools::debugfs;

mod app;
mod theme;
mod views;

fn main() {
    if !Uid::current().is_root() {
        eprintln!("Error: debugfs access requires root permissions");
        process::exit(1);
    }

    if let Err(err) = debugfs::mount() {
        eprintln!("Error: failed to mount debugfs: {err}");
        process::exit(1);
    }

    app::run();
}
