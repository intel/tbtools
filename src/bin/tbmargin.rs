// Run receiver lane margining on USB4 port and retimers.
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use std::io::{self, IsTerminal, Result};
use std::process;

use ansi_term::Colour::{Green, Red};
use clap::{self, Parser};
use nix::unistd::Uid;

use tbtools::{
    debugfs,
    margining::{
        Caps, IndependentTiming, IndependentVoltage, LaneResult, Lanes, Margin, Margining, Mode,
        ResultValue, Results, Test,
    },
    util, Address,
};

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Run receiver lane margining on USB4 port", long_about = None)]
struct Args {
    /// Domain number
    #[arg(short, long, default_value_t = 0)]
    domain: u8,
    /// Route string of the device
    #[arg(value_parser = util::parse_route, short, long)]
    route: u64,
    /// Lane 0 adapter number (1 - 64)
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..64))]
    adapter: u16,
    /// Retimer index if running on retimer
    #[arg(short, long, value_parser = clap::value_parser!(u16).range(1..6))]
    index: Option<u16>,
    /// Show capabilities only, do not run margining
    #[arg(short, long, default_value_t = false)]
    caps: bool,
}

fn color_result(res: &ResultValue) -> String {
    if io::stdout().is_terminal() {
        match res {
            ResultValue::Ok(value) => Green.paint(format!("{:6.2}", value)).to_string(),
            ResultValue::Exceeds(value) => Red.paint(format!("{:6.2}", value)).to_string(),
        }
    } else {
        match res {
            ResultValue::Ok(value) => format!("{:6.2}", value),
            ResultValue::Exceeds(value) => format!("{:6.2}!", value),
        }
    }
}

fn color_counter(counter: u32) -> String {
    if io::stdout().is_terminal() {
        if counter > 0 {
            Red.paint(format!("{}", counter)).to_string()
        } else {
            Green.paint(format!("{}", counter)).to_string()
        }
    } else if counter > 0 {
        format!("{}!", counter)
    } else {
        format!("{}", counter)
    }
}

macro_rules! show_margin {
    ($res:ident, $l:expr, $m:expr) => {{
        let margins = match $m {
            Margin::Low | Margin::Left => $res.low_left_margin($l),
            Margin::High | Margin::Right => $res.high_right_margin($l),
        };
        let margin = $m.to_string();

        for (idx, result) in margins.iter().enumerate() {
            if let Some(result) = result {
                let (value, unit) = match result {
                    LaneResult::Timing(value) => (value, "UI"),
                    LaneResult::Voltage(value) => (value, "mV"),
                };
                println!(
                    "Lane {idx} {margin:6 }margin : {} {unit}",
                    color_result(value)
                );
            }
        }
    }};
}

macro_rules! show_errors {
    ($l:expr, $res:ident) => {{
        println!(
            "Lane {} margin errors : {}",
            if $l == Lanes::Lane0 { 0 } else { 1 },
            color_counter($res.error_counter($l).0)
        );
    }};
}

fn show_hardware_results(lane: Lanes, test: &Test, margin: Option<&Margin>, results: &Results) {
    match *test {
        Test::Voltage => match lane {
            Lanes::Lane0 | Lanes::All => match margin {
                Some(m @ Margin::Low) => show_margin!(results, lane, m),
                Some(m @ Margin::High) => show_margin!(results, lane, m),
                None => {
                    show_margin!(results, Lanes::Lane0, Margin::Low);
                    show_margin!(results, Lanes::Lane0, Margin::High);
                }
                _ => panic!("Unsupported voltage margin"),
            },
            Lanes::Lane1 => match margin {
                Some(m @ Margin::Low) => show_margin!(results, lane, m),
                Some(m @ Margin::High) => show_margin!(results, lane, m),
                None => {
                    show_margin!(results, lane, Margin::Low);
                    show_margin!(results, lane, Margin::High);
                }
                _ => panic!("Unsupported voltage margin"),
            },
        },
        Test::Time => match lane {
            Lanes::Lane0 | Lanes::All => match margin {
                Some(m @ Margin::Left) => show_margin!(results, lane, m),
                Some(m @ Margin::Right) => show_margin!(results, lane, m),
                None => {
                    show_margin!(results, Lanes::Lane0, Margin::Left);
                    show_margin!(results, Lanes::Lane0, Margin::Right);
                }
                _ => panic!("Unsupported time margin"),
            },
            Lanes::Lane1 => match margin {
                Some(m @ Margin::Left) => show_margin!(results, lane, m),
                Some(m @ Margin::Right) => show_margin!(results, lane, m),
                None => {
                    show_margin!(results, lane, Margin::Left);
                    show_margin!(results, lane, Margin::Right);
                }
                _ => panic!("Unsupported time margin"),
            },
        },
    }
}

fn show_software_results(lane: Lanes, test: &Test, results: &Results) {
    match *test {
        Test::Voltage => match lane {
            Lanes::Lane0 | Lanes::All => show_errors!(Lanes::Lane0, results),
            Lanes::Lane1 => show_errors!(lane, results),
        },
        Test::Time => match lane {
            Lanes::Lane0 | Lanes::All => show_errors!(Lanes::Lane0, results),
            Lanes::Lane1 => show_errors!(lane, results),
        },
    }
}

fn show_results(lane: Lanes, test: &Test, mode: &Mode, margin: Option<&Margin>, results: &Results) {
    if *mode == Mode::Hardware {
        show_hardware_results(lane, test, margin, results)
    } else {
        show_software_results(lane, test, results)
    }
}

fn show_caps(caps: &Caps) {
    println!(
        "Hardware margining         : {}",
        if caps.hardware { "Yes" } else { "No" }
    );
    println!(
        "Software margining         : {}",
        if caps.software { "Yes" } else { "No" }
    );
    println!(
        "Multi-lane margining       : {}",
        if caps.all_lanes { "Yes" } else { "No" }
    );
    println!(
        "Time margining             : {}",
        if caps.time.is_some() { "Yes" } else { "No" }
    );
    println!(
        "Maximum voltage offset     : {} mV",
        caps.max_voltage_offset
    );
    println!("Voltage margin steps       : {}", caps.voltage_steps);
    println!(
        "Independent voltage margins: {}",
        match caps.independent_voltage_margins {
            tbtools::margining::IndependentVoltage::Gen23Minimum => "No (minimum)",
            tbtools::margining::IndependentVoltage::Gen23Both => "Yes (both)",
            tbtools::margining::IndependentVoltage::Gen23Either => "Yes (either)",
            tbtools::margining::IndependentVoltage::Gen4Minimum => "No (minimum)",
            tbtools::margining::IndependentVoltage::Gen4Both => "Yes (both)",
        }
    );

    if let Some(time) = caps.time {
        println!(
            "Destructive time margining : {}",
            if time.destructive { "Yes" } else { "No" }
        );
        println!("Maximum time offset        : {} UI", time.max_offset);
        println!("Time margin steps          : {}", time.steps);
    }
}

fn run_margining(args: &Args, margining: &mut Margining) -> Result<()> {
    let caps = margining.caps();

    show_caps(&caps);

    if args.caps {
        return Ok(());
    }

    // Try with the hardware mode but if not supported then software.
    if caps.hardware {
        margining.set_mode(&Mode::Hardware);
    } else {
        margining.set_mode(&Mode::Software);
    }

    let tests = if caps.time.is_some_and(|time| !time.destructive) {
        vec![Test::Voltage, Test::Time]
    } else {
        vec![Test::Voltage]
    };

    let lanes = if caps.all_lanes {
        vec![Lanes::All]
    } else {
        vec![Lanes::Lane0, Lanes::Lane1]
    };

    println!();

    for (index, test) in tests.iter().enumerate() {
        let margins: Vec<Margin> = match test {
            Test::Voltage => {
                println!("Running {} voltage margining", margining.mode());
                if caps.independent_voltage_margins == IndependentVoltage::Gen23Either {
                    vec![Margin::Low, Margin::High]
                } else {
                    vec![]
                }
            }
            Test::Time => {
                println!("Running {} time margining", margining.mode());
                if caps
                    .time
                    .is_some_and(|time| time.independent_margins == IndependentTiming::Gen23Either)
                {
                    vec![Margin::Left, Margin::Right]
                } else {
                    vec![]
                }
            }
        };

        margining.set_test(test);

        for &lane in lanes.iter() {
            margining.set_lanes(lane);

            if !margins.is_empty() {
                for margin in &margins {
                    margining.set_margin(margin);
                    show_results(
                        lane,
                        test,
                        &margining.mode(),
                        Some(margin),
                        &margining.run()?,
                    );
                }
            } else {
                show_results(lane, test, &margining.mode(), None, &margining.run()?);
            }
        }

        if index < tests.len() - 1 {
            println!();
        }
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

    if !Uid::current().is_root() {
        eprintln!("Error: debugfs access requires root permissions!");
        process::exit(1);
    }

    if let Err(err) = debugfs::mount() {
        eprintln!("Error: failed to mount debugfs: {}", err);
        process::exit(1);
    }

    let address = if let Some(index) = args.index {
        Address::Retimer {
            domain: args.domain,
            route: args.route,
            adapter: args.adapter as u8,
            index: index as u8,
        }
    } else {
        Address::Adapter {
            domain: args.domain,
            route: args.route,
            adapter: args.adapter as u8,
        }
    };

    let mut margining = match Margining::new(&address) {
        Err(err) => {
            eprintln!("Error: failed to initialize margining: {}", err);
            process::exit(1);
        }
        Ok(margining) => margining,
    };

    if let Err(err) = run_margining(&args, &mut margining) {
        eprintln!("Error: failed to run margining {}", err);
        process::exit(1);
    }
}
