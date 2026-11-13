// Manage USB4STREAM
//
// Copyright (C) 2026, Intel Corporation
// Author: Alan Borzeszkowski <alan.borzeszkowski@linux.intel.com>

use clap::{Parser, Subcommand};
use csv::Writer;
use nix::unistd::Uid;
use nu_ansi_term::Color::{Cyan, Green, Yellow};
use nu_ansi_term::Style;
use std::collections::HashSet;
use std::io::{self, IsTerminal};
use std::process;
use tbtools::stream::{HOPID_AUTO_ALLOC, Pairing, Stream, StreamDevice};

const UNKNOWN: &str = "Unknown";

#[derive(Parser, Debug)]
#[command(version)]
#[command(about = "Manage USB4 Streams and stream devices", long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List Streams and their stream devices
    List {
        /// Output suitable for scripts
        #[arg(short = 'S', long)]
        script: bool,
    },
    /// Add and configure stream device automatically
    Auto {
        /// Stream group name, required if more than one Stream is available
        stream_name: Option<String>,
    },
    /// Add a stream device to a Stream
    Add {
        /// Stream group name
        stream_name: String,
        /// Create stream device with this name
        stream_dev_name: String,
    },
    /// Remove stream devices, or whole Stream if no device name is given.
    ///
    /// Stream is also cleaned up once last stream device is removed.
    Remove {
        /// Remove all Streams along with their stream devices
        #[arg(long, conflicts_with_all = ["stream_name", "stream_dev_names"])]
        all: bool,
        /// Stream group name
        #[arg(required_unless_present = "all")]
        stream_name: Option<String>,
        /// Names of stream devices to remove
        stream_dev_names: Vec<String>,
    },
}

fn list_script(streams: &[Stream]) -> io::Result<()> {
    let mut writer = Writer::from_writer(io::stdout());
    writer.write_record([
        "stream",
        "connected",
        "group",
        "device",
        "in_hopid",
        "out_hopid",
        "devnode",
        "peer_vendor_name",
        "peer_device_name",
    ])?;

    for stream in streams {
        let (peer_vendor, peer_device) = get_peer(stream).unwrap_or_default();

        let group = stream.group_exists();
        let sdevs = if group { stream.devices()? } else { Vec::new() };

        let record = |sdev: Option<&StreamDevice>| {
            vec![
                stream.kernel_name().to_string(),
                stream.is_connected().to_string(),
                group.to_string(),
                sdev.map(|s| s.name().to_string()).unwrap_or_default(),
                sdev.map(|s| s.in_hopid().to_string()).unwrap_or_default(),
                sdev.map(|s| s.out_hopid().to_string()).unwrap_or_default(),
                sdev.map(|s| s.devnode()).unwrap_or_default(),
                peer_vendor.clone(),
                peer_device.clone(),
            ]
        };

        if sdevs.is_empty() {
            writer.write_record(record(None))?;
        }
        for sdev in &sdevs {
            writer.write_record(record(Some(sdev)))?;
        }
    }

    writer.flush()
}

fn paint(style: impl Into<Style>, s: &str) -> String {
    if io::stdout().is_terminal() {
        style.into().paint(s).to_string()
    } else {
        s.to_string()
    }
}

fn get_peer(stream: &Stream) -> Option<(String, String)> {
    let xd = stream.peer()?;
    Some((
        xd.vendor_name().unwrap_or_else(|| UNKNOWN.to_string()),
        xd.device_name().unwrap_or_else(|| UNKNOWN.to_string()),
    ))
}

fn stream_peer(stream: &Stream) -> String {
    match get_peer(stream) {
        Some((vendor, device)) => format!("{vendor} {device}"),
        None => "peer device not found".to_string(),
    }
}

fn stream_desc(stream: &Stream) -> String {
    let name = paint(Style::new().bold(), stream.kernel_name());

    if stream.is_connected() {
        format!(
            "Stream: {name}, Peer: {}",
            paint(Cyan, &stream_peer(stream))
        )
    } else {
        format!("Stream: {name}, {}", paint(Yellow, "configfs ONLY"))
    }
}

fn stream_desc_auto(stream: &Stream) -> String {
    if !stream.is_connected() {
        return paint(Style::new().dimmed(), stream.kernel_name());
    }

    format!(
        "{}, {}",
        paint(Style::new().bold(), stream.kernel_name()),
        paint(Cyan, &stream_peer(stream))
    )
}

fn format_hopid(hopid: i32) -> String {
    match hopid {
        0 => paint(Yellow, "unset"),
        HOPID_AUTO_ALLOC => paint(Yellow, &format!("{hopid} (auto allocation mode)")),
        hopid => paint(Green, &hopid.to_string()),
    }
}

fn print_stream_devices(stream: &Stream) -> io::Result<()> {
    let sdevs = if stream.group_exists() {
        stream.devices()?
    } else {
        Vec::new()
    };

    if sdevs.is_empty() {
        println!("  {}", paint(Style::new().dimmed(), "<none>"));
    }
    for sdev in sdevs {
        println!("  {}", paint(Style::new().bold(), sdev.name()));
        println!("    In HopID:     {}", format_hopid(sdev.in_hopid()));
        println!("    Out HopID:    {}", format_hopid(sdev.out_hopid()));
        println!("    Device Node:  {}", sdev.devnode());
    }
    Ok(())
}

fn list(streams: &[Stream]) -> io::Result<()> {
    if streams.is_empty() {
        println!("No Streams found.");
        return Ok(());
    }

    for stream in streams {
        println!("{}", stream_desc(stream));
        print_stream_devices(stream)?;
    }
    Ok(())
}

fn create_stream_device(
    stream: &Stream,
    name: Option<&str>,
) -> io::Result<(StreamDevice, Pairing)> {
    let sdev = stream.device_add(name)?;

    match sdev.configure() {
        Ok(pairing) => Ok((sdev, pairing)),
        Err(e) => {
            let dev_name = sdev.name().to_string();
            if let Err(rm_err) = sdev.remove() {
                eprintln!("Warning: failed to remove stream device '{dev_name}': {rm_err}");
            }
            Err(e)
        }
    }
}

fn add_stream(stream: &Stream) {
    if let Err(e) = stream.add() {
        eprintln!(
            "Error: failed to create configfs group for Stream '{}': {e}",
            stream.kernel_name()
        );
        process::exit(1);
    }
}

fn find_stream<'a>(streams: &'a [Stream], name: &str) -> &'a Stream {
    streams
        .iter()
        .find(|s| s.kernel_name() == name)
        .unwrap_or_else(|| {
            eprintln!("Error: Stream '{name}' not found");
            process::exit(1);
        })
}

fn select_stream<'a>(streams: &'a [Stream], name: Option<&str>) -> &'a Stream {
    if let Some(name) = name {
        return find_stream(streams, name);
    }

    // Select Stream that is actually backed by USB4 cable
    let connected: Vec<&Stream> = streams.iter().filter(|s| s.is_connected()).collect();
    match connected.as_slice() {
        [stream] => stream,
        [] => {
            eprintln!("Error: peer host not found, connect it with Thunderbolt/USB4 cable");
            process::exit(1);
        }
        _ => {
            println!(
                "More than one Stream available, select one with 'tbstream auto <stream_name>':"
            );
            for stream in &connected {
                println!("{}", stream_desc(stream));
            }
            process::exit(1);
        }
    }
}

fn remove_stream(stream: &Stream) -> bool {
    let stream_name = stream.kernel_name();

    if let Err(e) = stream.remove() {
        eprintln!("Error: failed to remove Stream '{stream_name}': {e}");
        return false;
    }
    println!(
        "{} Stream '{stream_name}' configfs group and all its stream devices",
        paint(Green, "Removed")
    );
    true
}

fn remove_all_streams(streams: &[Stream]) -> bool {
    let groups: Vec<&Stream> = streams.iter().filter(|s| s.group_exists()).collect();

    if groups.is_empty() {
        println!("No Streams to remove.");
        return true;
    }

    let mut ok = true;
    for stream in groups {
        ok &= remove_stream(stream);
    }
    ok
}

fn main() {
    let args = Args::parse();

    if !Uid::current().is_root() {
        eprintln!("Error: root access required!");
        process::exit(1);
    }

    if !Stream::is_supported() {
        if let Err(err) = Stream::mount_configfs() {
            eprintln!("Error: failed to mount configfs: {err}");
            process::exit(1);
        }
        if !Stream::is_supported() {
            eprintln!(
                "Error: USB4STREAM not supported. Check if thunderbolt_stream module is present."
            );
            process::exit(1);
        }
    }

    let streams = Stream::find_all().unwrap_or_else(|e| {
        eprintln!("Error: failed to enumerate Streams: {e}");
        process::exit(1);
    });

    match args.command {
        Commands::List { script } => {
            let result = if script {
                list_script(&streams)
            } else {
                list(&streams)
            };
            if let Err(e) = result {
                eprintln!("Error: {e}");
                process::exit(1);
            }
        }
        Commands::Auto { stream_name } => {
            let stream = select_stream(&streams, stream_name.as_deref());

            if !stream.group_exists() {
                println!(
                    "Stream '{}' configfs group not found, creating..",
                    stream.kernel_name()
                );
                add_stream(stream);
            }

            match create_stream_device(stream, None) {
                Ok((sdev, Pairing::Paired)) => println!(
                    "Stream device '{}' ({}) {} with {}",
                    sdev.name(),
                    sdev.devnode(),
                    paint(Green, "paired"),
                    stream_desc_auto(stream)
                ),
                Ok((sdev, Pairing::Pending)) => println!(
                    "Stream device '{}' {}.\nRun 'tbstream auto' on peer {}",
                    sdev.name(),
                    paint(Green, "created"),
                    stream_desc_auto(stream)
                ),
                Err(e) => {
                    eprintln!(
                        "Error: failed to add stream device to Stream '{}': {e}",
                        stream.kernel_name()
                    );
                    process::exit(1);
                }
            }
        }
        Commands::Add {
            stream_name,
            stream_dev_name,
        } => {
            let stream = find_stream(&streams, &stream_name);

            if stream.device_exists(&stream_dev_name) {
                eprintln!(
                    "Error: stream device '{stream_dev_name}' already exists under Stream '{stream_name}'"
                );
                process::exit(1);
            }

            if !stream.group_exists() {
                println!("Stream '{stream_name}' configfs group not found, creating..");
                add_stream(stream);
            }
            match create_stream_device(stream, Some(&stream_dev_name)) {
                Ok((sdev, _)) => {
                    println!(
                        "{} stream device '{}' to Stream '{stream_name}'.",
                        paint(Green, "Added"),
                        sdev.name()
                    )
                }
                Err(e) => {
                    eprintln!(
                        "Error: failed to add stream device '{stream_dev_name}' to Stream '{stream_name}': {e}"
                    );
                    process::exit(1);
                }
            }
        }
        Commands::Remove {
            all,
            stream_name,
            stream_dev_names,
        } => {
            if all {
                if !remove_all_streams(&streams) {
                    process::exit(1);
                }
                return;
            }

            // At this point stream_name is required so its safe to unwrap
            let stream_name = stream_name.unwrap();

            let stream = find_stream(&streams, &stream_name);

            if !stream.group_exists() {
                eprintln!("Error: Stream '{stream_name}' configfs group does not exist");
                process::exit(1);
            }

            // Remove whole Stream group if stream devices are not specified
            if stream_dev_names.is_empty() {
                if !remove_stream(stream) {
                    process::exit(1);
                }
                return;
            }

            let mut seen = HashSet::new();
            let mut failed = false;

            for dev_name in &stream_dev_names {
                // Skip duplicate stream device names
                if !seen.insert(dev_name.as_str()) {
                    continue;
                }

                match stream.device(dev_name).and_then(|sdev| sdev.remove()) {
                    Ok(()) => println!(
                        "{} stream device '{dev_name}' from Stream '{stream_name}'",
                        paint(Green, "Removed")
                    ),
                    Err(e) => {
                        eprintln!(
                            "Error: failed to remove stream device '{dev_name}' from Stream '{stream_name}': {e}"
                        );
                        failed = true;
                    }
                }
            }

            // Cleanup Stream group once its last stream device is removed
            match stream.devices() {
                Ok(sdevs) if sdevs.is_empty() => {
                    println!(
                        "Stream '{stream_name}' has no stream devices left. {}",
                        paint(Green, "Removing..")
                    );
                    if let Err(e) = stream.remove() {
                        eprintln!("Error: failed to remove Stream '{stream_name}': {e}");
                        failed = true;
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!(
                        "Error: failed to read stream devices of Stream '{stream_name}': {e}"
                    );
                    failed = true;
                }
            }

            if failed {
                process::exit(1);
            }
        }
    }
}
