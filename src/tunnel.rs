// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2025, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Implements functions to read tunnel configuration through `debugfs`.
//!
//! # Examples
//! Below is an example how to dump all tunnels going through devices in `devices`.
//! ```no_run
//! # use std::io;
//! # use tbtools::Address;
//! use tbtools::tunnel::Tunnel;
//!
//! # fn main() -> io::Result<()> {
//! # let mut devices: Vec<_> = tbtools::find_devices(None)?;
//! // Note devices need to have read_adapters() called and for each involved adapter read_paths()
//! // so that Tunnel::discover() can work.
//! for device in &devices {
//!     if !device.is_router() {
//!         continue;
//!      }
//!      if let Some(adapters) = device.adapters() {
//!          for adapter in adapters {
//!              if !adapter.is_enabled() {
//!                  continue;
//!              }
//!              if let Some(tunnels) = Tunnel::discover(device, adapter, &devices) {
//!                 // Do something with the tunnels
//!              }
//!         }
//!     }
//! }
//! # Ok(())
//! # }

use crate::{
    Device,
    debugfs::{self, Adapter, PathEntry},
};
use std::fmt::{self, Display};

const USB3_HOPID: u16 = 8;
const DP_AUX_TX_HOPID: u16 = 8;
const DP_AUX_RX_HOPID: u16 = 8;
const DP_VIDEO_HOPID: u16 = 9;
const PCIE_HOPID: u16 = 8;

/// Type of the tunnel.
#[derive(Copy, Clone, PartialEq)]
pub enum Type {
    /// USB 3.x GenX tunnel.
    Usb3,
    /// DisplayPort tunnel.
    DisplayPort,
    /// PCIe tunnel.
    Pcie,
    /// Host-to-host tunnel.
    Dma,
}

impl Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            Self::Usb3 => "USB 3",
            Self::DisplayPort => "DisplayPort",
            Self::Pcie => "PCIe",
            Self::Dma => "DMA",
        };
        write!(f, "{s}")
    }
}

/// Single extracted `Path` segment.
///
/// This holds the actual path register entry along with the actual routers and adapters involved.
pub struct Hop<'a> {
    entry: PathEntry,
    device: &'a Device,
}

impl<'a> Hop<'a> {
    fn new(path_entry: &PathEntry, device: &'a Device) -> Self {
        Self {
            entry: *path_entry,
            device,
        }
    }

    /// Returns the actual path register space entry.
    pub fn entry(&self) -> &PathEntry {
        &self.entry
    }

    /// Returns the device whose adapters are involved in this routing entry.
    pub fn device(&self) -> &'a Device {
        self.device
    }

    /// Returns the input adapter whose routing table this entry belongs to.
    pub fn in_adapter(&self) -> &'a Adapter {
        self.device.adapter(self.entry.in_adapter()).unwrap()
    }

    /// Returns the output adapter where the packets are routed to.
    pub fn out_adapter(&self) -> &'a Adapter {
        self.device.adapter(self.entry.out_adapter()).unwrap()
    }
}

/// Uni-directional path from source to destination.
///
/// This describes one uni-directional path from source adapter to destination adapter through the
/// USB4 fabric.
pub struct Path<'a> {
    name: String,
    hops: Vec<Hop<'a>>,
}

impl<'a> Path<'a> {
    fn new(name: &str, hops: Vec<Hop<'a>>) -> Self {
        Self {
            name: String::from(name),
            hops,
        }
    }

    /// Returns descriptive name of this path.
    ///
    /// Can be for example `AUX TX` and so on.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns starting device of this path.
    pub fn src_device(&self) -> &'a Device {
        self.hops[0].device()
    }

    /// Returns the adapter where this path starts.
    pub fn src_adapter(&self) -> &'a Adapter {
        self.hops[0].in_adapter()
    }

    /// Returns ending device of this path.
    pub fn dst_device(&self) -> &'a Device {
        self.hops.iter().last().unwrap().device()
    }

    /// Returns the adapter where this path ends.
    pub fn dst_adapter(&self) -> &'a Adapter {
        self.hops.iter().last().unwrap().out_adapter()
    }

    /// Returns all the segments in this path.
    pub fn hops(&self) -> &[Hop] {
        &self.hops
    }

    fn dst_hop(&self) -> u16 {
        self.hops.iter().last().unwrap().entry().out_hop()
    }
}

/// Describes one tunnel in the domain.
///
/// A Tunnel is formed from one or more [`Paths`](Path). All these together are then used as
/// "virtual" wire for the underlying native protocol. It is also possible to have "null" tunnels
/// between hosts which allow software traffic to be transferred between them.
pub struct Tunnel<'a> {
    kind: Type,
    paths: Vec<Path<'a>>,
}

impl<'a> Tunnel<'a> {
    /// Discovers a tunnel starting from source adapter `adapter`.
    ///
    /// The `adapter` must be enabled belong to `device` and `devices` must contain all the devices
    /// in the domain or at least the ones that the tunnel passes through.
    pub fn discover(
        device: &'a Device,
        adapter: &'a Adapter,
        devices: &'a [Device],
    ) -> Option<Vec<Self>> {
        if !adapter.is_enabled() {
            return None;
        }

        match adapter.kind() {
            debugfs::Type::Usb3Down => Self::discover_usb3(device, adapter, devices),
            debugfs::Type::DisplayPortIn => Self::discover_dp(device, adapter, devices),
            debugfs::Type::PcieDown => Self::discover_pcie(device, adapter, devices),
            debugfs::Type::HostInterface => Self::discover_dma(device, adapter, devices),
            _ => None,
        }
    }

    fn discover_usb3(
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        devices: &'a [Device],
    ) -> Option<Vec<Self>> {
        let mut paths = Vec::new();

        let down_path =
            Self::discover_path("USB 3 Down", src_device, src_adapter, USB3_HOPID, devices)?;
        let up_path = Self::discover_path(
            "USB 3 Up",
            down_path.dst_device(),
            down_path.dst_adapter(),
            USB3_HOPID,
            devices,
        )?;
        paths.push(down_path);
        paths.push(up_path);

        Some(vec![Self {
            kind: Type::Usb3,
            paths,
        }])
    }

    fn discover_dp(
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        devices: &'a [Device],
    ) -> Option<Vec<Self>> {
        let mut paths = Vec::new();

        let video_path =
            Self::discover_path("Video", src_device, src_adapter, DP_VIDEO_HOPID, devices)?;
        let aux_tx_path =
            Self::discover_path("AUX TX", src_device, src_adapter, DP_AUX_TX_HOPID, devices)?;
        let aux_rx_path = Self::discover_path(
            "AUX RX",
            aux_tx_path.dst_device(),
            aux_tx_path.dst_adapter(),
            DP_AUX_RX_HOPID,
            devices,
        )?;
        paths.push(video_path);
        paths.push(aux_tx_path);
        paths.push(aux_rx_path);

        Some(vec![Self {
            kind: Type::DisplayPort,
            paths,
        }])
    }

    fn discover_pcie(
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        devices: &'a [Device],
    ) -> Option<Vec<Self>> {
        let mut paths = Vec::new();

        let down_path =
            Self::discover_path("PCIe Down", src_device, src_adapter, PCIE_HOPID, devices)?;
        let up_path = Self::discover_path(
            "PCIe Up",
            down_path.dst_device(),
            down_path.dst_adapter(),
            PCIE_HOPID,
            devices,
        )?;
        paths.push(down_path);
        paths.push(up_path);

        Some(vec![Self {
            kind: Type::Pcie,
            paths,
        }])
    }

    fn discover_one_dma(
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        ring: u16,
        devices: &'a [Device],
    ) -> Option<Self> {
        let mut paths = Vec::new();

        if let Some(down_path) =
            Self::discover_path("DMA TX", src_device, src_adapter, ring, devices)
        {
            paths.push(down_path);
        }

        // For the RX direction, look at any XDomain device and find a tunnel that ends up using
        // the same ring. We know that Linux always uses same ring number for both directions so
        // that's our RX path then.
        'rx_path: for device in devices {
            if !device.is_xdomain() {
                continue;
            }

            if let Some(parent) = device.parent_from(devices)
                && let Some(downstream_adapter) = parent.downstream_adapter(device)
            {
                let downstream_adapter = parent.adapter(downstream_adapter)?;
                let min_hop = downstream_adapter.min_hop()?;
                let max_hop = downstream_adapter.max_hop()?;

                for hop in min_hop..=max_hop {
                    if let Some(up_path) =
                        Self::discover_path("DMA RX", parent, downstream_adapter, hop, devices)
                    {
                        if up_path.dst_hop() == ring {
                            paths.push(up_path);
                            break 'rx_path;
                        }
                    }
                }
            }
        }

        if paths.is_empty() {
            return None;
        }

        Some(Self {
            kind: Type::Dma,
            paths,
        })
    }

    fn discover_dma(
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        devices: &'a [Device],
    ) -> Option<Vec<Self>> {
        let mut tunnels = Vec::new();

        let min_hop = src_adapter.min_hop()?;
        let max_hop = src_adapter.max_hop()?;

        // We don't have visibility for the rings but look at the host interface path entries and
        // if we find enabled, assume this is DMA tunnel.
        for hopid in min_hop..=max_hop {
            if let Some(tunnel) = Self::discover_one_dma(src_device, src_adapter, hopid, devices) {
                tunnels.push(tunnel);
            }
        }

        Some(tunnels)
    }

    fn discover_path(
        name: &str,
        src_device: &'a Device,
        src_adapter: &'a Adapter,
        in_hop: u16,
        devices: &'a [Device],
    ) -> Option<Path<'a>> {
        let mut in_hop = in_hop;
        let mut adapter = src_adapter;
        let mut device = src_device;
        let mut hops = Vec::new();

        while let Some(path_entry) = adapter.path(in_hop) {
            let hop = Hop::new(path_entry, device);
            let out_adapter = hop.out_adapter();
            let out_hop = hop.entry().out_hop();

            hops.push(hop);

            if out_adapter.is_upstream() {
                if let Some(dev) = out_adapter.upstream_device(devices) {
                    adapter = dev.adapter(dev.downstream_adapter(device)?)?;
                    device = dev;
                } else {
                    break;
                }
            } else if let Some(dev) = out_adapter.downstream_device(devices) {
                device = dev;
                adapter = device.adapter(dev.upstream_adapter()?)?;
            } else {
                break;
            }

            in_hop = out_hop;
        }

        if hops.is_empty() {
            return None;
        }

        Some(Path::new(name, hops))
    }

    /// Returns type of this tunnel.
    pub fn kind(&self) -> Type {
        self.kind
    }

    /// Returns starting point of this tunnel.
    pub fn src_device(&self) -> &Device {
        self.paths[0].src_device()
    }

    /// Returns starting adapter of this tunnel.
    pub fn src_adapter(&self) -> &Adapter {
        self.paths[0].src_adapter()
    }

    /// Returns ending point of this tunnel.
    pub fn dst_device(&self) -> &Device {
        self.paths[0].dst_device()
    }

    /// Returns ending adapter of this tunnel.
    pub fn dst_adapter(&self) -> &Adapter {
        self.paths[0].dst_adapter()
    }

    /// Returns the individual paths that make up this tunnel.
    pub fn paths(&self) -> &[Path] {
        &self.paths
    }
}
