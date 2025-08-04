// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use lazy_static::lazy_static;

use serde::{Serialize, Serializer};
use std::cmp::Ordering;
use std::ffi::OsStr;
use std::fmt::{self, Display};
use std::io::{self, Error, ErrorKind};
use std::path::PathBuf;

use regex::Regex;

use crate::{
    debugfs::{Adapter, Register},
    drom::Drom,
    usb4, util,
};

lazy_static! {
    static ref USB4_VERSION_RE: Regex = Regex::new(r"^(\d+)\.(\d+)").unwrap();
    static ref NVM_VERSION_RE: Regex = Regex::new(r"^([[:xdigit:]]+)\.([[:xdigit:]]+)").unwrap();
    static ref DOMAIN_RE: Regex = Regex::new(r"^domain(\d+)").unwrap();
    static ref RETIMER_RE: Regex = Regex::new(r"^(\d+)-(\d+):(\d+).(\d+)").unwrap();
    static ref ROUTER_RE: Regex = Regex::new(r"^(\d+)-(\d+)").unwrap();
    static ref SPEED_RE: Regex = Regex::new(r"(\d+).0 Gb/s").unwrap();
}

/// Describes type of the device.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum Kind {
    /// Device is host.
    Domain,
    /// Device is router.
    Router,
    /// Device is remote host.
    Xdomain,
    /// Device is retimer.
    Retimer,
    /// Device is not known.
    Unknown,
}

impl From<&str> for Kind {
    fn from(s: &str) -> Self {
        match s {
            "thunderbolt_domain" => Self::Domain,
            "thunderbolt_device" => Self::Router,
            "thunderbolt_xdomain" => Self::Xdomain,
            "thunderbolt_retimer" => Self::Retimer,
            _ => Self::Unknown,
        }
    }
}

impl Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::Domain => "thunderbolt_domain",
            Self::Router => "thunderbolt_device",
            Self::Xdomain => "thunderbolt_xdomain",
            Self::Retimer => "thunderbolt_retimer",
            Self::Unknown => "unknown",
        };
        write!(f, "{s}")
    }
}

/// Thunderbolt security level.
///
/// This is used to determine whether PCIe tunnel are created automatically or by user approval.
/// There is more information about these in the kernel [Thunderbolt/USB4 documentation].
///
/// [Thunderbolt/USB4 documentation]: https://docs.kernel.org/admin-guide/thunderbolt.html#security-levels-and-how-to-use-them
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum SecurityLevel {
    /// PCIe tunnel is created automatically.
    None,
    /// User approval is needed.
    User,
    /// User approval is needed and the device must match the stored challenge.
    Secure,
    /// Only DisplayPort and USB tunneling is done.
    DpOnly,
    /// Only one PCIe tunnel to first level USB controller is created.
    UsbOnly,
    /// PCIe tunneling is disabled by the BIOS/boot firmware.
    NoPcie,
    /// Unknown security level.
    Unknown,
}

impl From<&str> for SecurityLevel {
    fn from(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "user" => Self::User,
            "secure" => Self::Secure,
            "dponly" => Self::DpOnly,
            "usbonly" => Self::UsbOnly,
            "nopcie" => Self::NoPcie,
            _ => Self::Unknown,
        }
    }
}

/// Encodes hardware and firmware version numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
}

/// Represents a device on a Thunderbolt/USB4 bus.
///
/// Any type of device on a Thunderbolt/USB4 bus is represented by an instance of this object.
/// The actual type can be determined by calling [`kind()`](Self::kind()).
#[derive(Clone, Debug)]
pub struct Device {
    kernel_name: String,
    kind: Kind,
    domain: u32,
    route: u64,
    adapter_num: u8,
    index: u8,
    device: u16,
    vendor: u16,
    device_name: Option<String>,
    vendor_name: Option<String>,
    authorized: Option<bool>,
    unique_id: Option<String>,
    generation: Option<u8>,
    security_level: Option<SecurityLevel>,
    iommu: Option<bool>,
    deauthorization: Option<bool>,
    usb4_version: Option<Version>,
    rx_speed: Option<u32>,
    rx_lanes: Option<u32>,
    tx_speed: Option<u32>,
    tx_lanes: Option<u32>,
    nvm_version: Option<Version>,
    key: Option<String>,
    syspath: PathBuf,

    pub(crate) regs: Option<Vec<Register>>,
    pub(crate) adapters: Option<Vec<Adapter>>,
    pub(crate) drom: Option<Drom>,
}

impl Device {
    pub(crate) fn udev(&self) -> io::Result<udev::Device> {
        udev::Device::from_syspath(&self.syspath)
    }

    /// Returns kernel name of the device.
    pub fn kernel_name(&self) -> String {
        self.kernel_name.clone()
    }

    /// Returns formatted name of the device.
    pub fn name(&self) -> String {
        match self.kind {
            Kind::Domain => format!("Domain {}", self.domain_index()),
            Kind::Router | Kind::Xdomain => {
                format!(
                    "Domain {} Route {:x} {:04x}:{:04x}",
                    self.domain_index(),
                    self.route(),
                    self.vendor(),
                    self.device(),
                )
            }
            Kind::Retimer => {
                format!(
                    "Domain {} Route {:x} Adapter {} Index {} {:04x}:{:04x}",
                    self.domain_index(),
                    self.route(),
                    self.adapter_num(),
                    self.index(),
                    self.vendor(),
                    self.device(),
                )
            }
            _ => String::from("Unknown"),
        }
    }

    /// Returns type of this device.
    pub fn kind(&self) -> Kind {
        self.kind.clone()
    }

    /// Returns the domain number.
    pub fn domain_index(&self) -> u32 {
        self.domain
    }

    /// Returns `true` if this device is domain.
    pub fn is_domain(&self) -> bool {
        self.kind == Kind::Domain
    }

    /// Returns `true` if this device is XDomain.
    pub fn is_xdomain(&self) -> bool {
        self.kind == Kind::Xdomain
    }

    /// Returns `true` if this is router.
    pub fn is_router(&self) -> bool {
        self.kind == Kind::Router
    }

    /// Returns `true` if the device is host router
    pub fn is_host_router(&self) -> bool {
        self.is_router() && self.route == 0
    }

    /// Returns `true` if the device is device router
    pub fn is_device_router(&self) -> bool {
        self.is_router() && self.route > 0
    }

    /// Returns the domain this device belongs to
    pub fn domain(&self) -> Option<Self> {
        if self.is_domain() {
            return Self::parse(self.udev().ok()?);
        }

        let mut parent = self.parent()?;

        loop {
            if parent.is_domain() {
                return Some(parent);
            }

            parent = parent.parent()?
        }
    }

    /// Returns route string (`TopologyID`) of the device.
    pub fn route(&self) -> u64 {
        self.route
    }

    /// Returns device ID.
    pub fn device(&self) -> u16 {
        self.device
    }

    /// Returns vendor ID.
    pub fn vendor(&self) -> u16 {
        self.vendor
    }

    /// Returns name of the device from DROM if exists.
    pub fn device_name(&self) -> Option<String> {
        self.device_name.clone()
    }

    /// Returns name of the vendor from DROM if exists.
    pub fn vendor_name(&self) -> Option<String> {
        self.vendor_name.clone()
    }

    /// If this is retimer, returns the adapter number this is connected to.
    pub fn adapter_num(&self) -> u8 {
        self.adapter_num
    }

    /// If this is retimer, returns the retimer index this answers to.
    pub fn index(&self) -> u8 {
        self.index
    }

    /// Returns depth of the device in topology.
    pub fn depth(&self) -> u32 {
        (u64::BITS - self.route.leading_zeros()).div_ceil(usb4::ROUTE_SHIFT)
    }

    /// Returns Rx speed of the device in Mb/s.
    pub fn rx_speed(&self) -> Option<u32> {
        self.rx_speed
    }

    /// Returns Tx speed of the device in Mb/s.
    pub fn tx_speed(&self) -> Option<u32> {
        self.tx_speed
    }

    /// Returns number of Rx lanes the device is using.
    pub fn rx_lanes(&self) -> Option<u32> {
        self.rx_lanes
    }

    /// Returns number of Tx lanes the device is using.
    pub fn tx_lanes(&self) -> Option<u32> {
        self.tx_lanes
    }

    /// Returns NVM firmare version of the device
    pub fn nvm_version(&self) -> Option<Version> {
        self.nvm_version
    }

    /// Is the device authorized. Only applies on routers for others returns None
    pub fn authorized(&self) -> Option<bool> {
        self.authorized
    }

    /// Returns the key used for challenge.
    pub fn key(&self) -> Option<String> {
        self.key.clone()
    }

    /// Returns `true` if the challenge key is set.
    pub fn has_key(&self) -> bool {
        self.key().is_some()
    }

    /// Sets the challenge key.
    pub fn set_key(&mut self, key: &str) -> io::Result<()> {
        if self.kind != Kind::Router || !self.has_key() {
            return Err(Error::from(ErrorKind::InvalidData));
        } else {
            self.udev()?.set_attribute_value("key", key)?;
        }

        Ok(())
    }

    /// Authorize router. Any other device will result an error.
    pub fn authorize(&mut self, authorize: u32) -> io::Result<()> {
        if self.kind != Kind::Router {
            Err(Error::from(ErrorKind::InvalidData))
        } else {
            self.udev()?
                .set_attribute_value("authorized", format!("{authorize}"))
        }
    }

    /// Returns UUID of the device.
    pub fn unique_id(&self) -> Option<String> {
        self.unique_id.clone()
    }

    /// Returns Thunderbolt generation of the router. Can `1`..`4` or `None` if this is not a
    /// router.
    pub fn generation(&self) -> Option<u8> {
        self.generation
    }

    /// Returns USB4 version of the router. If the router predates USB4 returns `None`.
    pub fn usb4_version(&self) -> Option<Version> {
        self.usb4_version
    }

    /// Returns current security level of the domain.
    pub fn security_level(&self) -> Option<SecurityLevel> {
        self.security_level.clone()
    }

    /// Returns `true` if the domain supports de-authorization of the devices.
    pub fn deauthorization(&self) -> Option<bool> {
        self.deauthorization
    }

    /// Returns `true` if DMA is protected by an IOMMU.
    pub fn iommu_dma_protection(&self) -> Option<bool> {
        self.iommu
    }

    /// Returns path in sysfs for this device.
    pub fn sysfs_path(&self) -> PathBuf {
        self.syspath.clone()
    }

    fn parse_speed(value: Option<&OsStr>) -> Option<u32> {
        if let Some(speed) = value {
            let caps = SPEED_RE.captures(speed.to_str()?)?;
            caps[1].parse::<u32>().ok()
        } else {
            None
        }
    }

    fn parse_lanes(value: Option<&OsStr>) -> Option<u32> {
        if let Some(lanes) = value {
            lanes.to_str()?.parse::<u32>().ok()
        } else {
            None
        }
    }

    // The device really has this many fields so the constructor needs all these parameters. For
    // this reason we disable the lint warning here. We may change this in the future.
    #[allow(clippy::too_many_arguments)]
    fn new(
        kernel_name: String,
        kind: Kind,
        domain: u32,
        route: u64,
        adapter_num: u8,
        index: u8,
        vendor: u16,
        device: u16,
        device_name: Option<String>,
        vendor_name: Option<String>,
        authorized: Option<bool>,
        unique_id: Option<String>,
        generation: Option<u8>,
        security_level: Option<SecurityLevel>,
        iommu: Option<bool>,
        deauthorization: Option<bool>,
        usb4_version: Option<Version>,
        rx_speed: Option<u32>,
        rx_lanes: Option<u32>,
        tx_speed: Option<u32>,
        tx_lanes: Option<u32>,
        nvm_version: Option<Version>,
        key: Option<String>,
        syspath: PathBuf,
    ) -> Self {
        Device {
            kernel_name,
            kind,
            domain,
            route,
            adapter_num,
            index,
            vendor,
            device,
            device_name,
            vendor_name,
            authorized,
            unique_id,
            generation,
            security_level,
            iommu,
            deauthorization,
            usb4_version,
            rx_speed,
            rx_lanes,
            tx_speed,
            tx_lanes,
            nvm_version,
            key,
            syspath,
            regs: None,
            adapters: None,
            drom: None,
        }
    }

    pub(crate) fn parse(udev: udev::Device) -> Option<Self> {
        let kind = Kind::from(udev.devtype()?.to_str()?);

        let vendor_id = udev
            .attribute_value("vendor")
            .and_then(|v| util::parse_hex(v.to_str()?))
            .unwrap_or(0);
        let device_id = udev
            .attribute_value("device")
            .and_then(|v| util::parse_hex(v.to_str()?))
            .unwrap_or(0);

        let device_name = udev
            .attribute_value("device_name")
            .and_then(|n| n.to_str())
            .map(String::from);
        let vendor_name = udev
            .attribute_value("vendor_name")
            .and_then(|n| n.to_str())
            .map(String::from);
        let authorized = udev
            .attribute_value("authorized")
            .and_then(|n| n.to_str())
            .map(|n| n.parse::<u32>().unwrap_or(0) > 0);
        let unique_id = udev
            .attribute_value("unique_id")
            .and_then(|n| n.to_str())
            .map(String::from);
        let generation = udev
            .attribute_value("generation")
            .and_then(|n| n.to_str())
            .map(|n| n.parse::<u8>().unwrap());

        let security_level = udev
            .attribute_value("security")
            .and_then(|n| n.to_str())
            .map(String::from)
            .map(|security_level| SecurityLevel::from(security_level.as_str()));

        let iommu = udev
            .attribute_value("iommu_dma_protection")
            .and_then(|n| n.to_str())
            .map(|n| n.parse::<u32>().unwrap_or(0) > 0);
        let deauthorization = udev
            .attribute_value("deauthorization")
            .and_then(|n| n.to_str())
            .map(|n| n.parse::<u32>().unwrap_or(0) > 0);

        let usb4_version = if generation >= Some(4) {
            let version = udev.property_value("USB4_VERSION")?.to_str().unwrap();
            let caps = USB4_VERSION_RE.captures(version).unwrap();
            let major = util::parse_hex::<u8>(&caps[1]).unwrap();
            let minor = util::parse_hex::<u8>(&caps[2]).unwrap();

            Some(Version { major, minor })
        } else {
            None
        };

        let rx_speed = Self::parse_speed(udev.attribute_value("rx_speed"));
        let rx_lanes = Self::parse_lanes(udev.attribute_value("rx_lanes"));
        let tx_speed = Self::parse_speed(udev.attribute_value("tx_speed"));
        let tx_lanes = Self::parse_lanes(udev.attribute_value("tx_lanes"));

        let nvm_version = udev
            .attribute_value("nvm_version")
            .and_then(|n| n.to_str())
            .and_then(|n| {
                let caps = NVM_VERSION_RE.captures(n)?;
                let major = util::parse_hex::<u8>(&caps[1])?;
                let minor = util::parse_hex::<u8>(&caps[2])?;

                Some(Version { major, minor })
            });

        let key = udev
            .attribute_value("key")
            .and_then(|n| n.to_str())
            .map(String::from);

        let kernel_name = String::from(udev.sysname().to_str()?);
        let domain: u32;
        let mut route: u64 = 0;
        let mut adapter_num: u8 = 0;
        let mut index: u8 = 0;

        match kind {
            Kind::Domain => {
                let caps = DOMAIN_RE.captures(&kernel_name).unwrap();
                domain = caps[1].parse().unwrap();
            }

            Kind::Retimer => {
                let caps = RETIMER_RE.captures(&kernel_name).unwrap();
                domain = caps[1].parse().unwrap_or(0);
                route = util::parse_hex::<u64>(&caps[2]).unwrap();
                adapter_num = caps[3].parse().unwrap();
                index = caps[4].parse().unwrap();
            }

            _ => {
                let caps = ROUTER_RE.captures(&kernel_name).unwrap();
                domain = caps[1].parse().unwrap_or(0);
                route = util::parse_hex::<u64>(&caps[2]).unwrap_or(0);
            }
        }

        let syspath = udev.syspath().to_path_buf();

        Some(Self::new(
            kernel_name,
            kind,
            domain,
            route,
            adapter_num,
            index,
            vendor_id,
            device_id,
            device_name,
            vendor_name,
            authorized,
            unique_id,
            generation,
            security_level,
            iommu,
            deauthorization,
            usb4_version,
            rx_speed,
            rx_lanes,
            tx_speed,
            tx_lanes,
            nvm_version,
            key,
            syspath,
        ))
    }
}

impl Eq for Device {}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.domain == other.domain && self.route == other.route
    }
}

impl Ord for Device {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.domain < other.domain {
            return Ordering::Less;
        } else if self.domain > other.domain {
            return Ordering::Greater;
        } else if self.route < other.route {
            return Ordering::Less;
        } else if self.route > other.route {
            return Ordering::Greater;
        }
        Ordering::Equal
    }
}

impl PartialOrd for Device {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Bus address of a device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Address {
    /// Address a Domain.
    Domain { domain: u8 },
    /// Address a Router.
    Router { domain: u8, route: u64 },
    /// Address an Inter-Domain connection
    Xdomain { domain: u8, route: u64 },
    /// Address an Adapter of a Router.
    Adapter { domain: u8, route: u64, adapter: u8 },
    /// Address a Retimer.
    Retimer {
        domain: u8,
        route: u64,
        adapter: u8,
        index: u8,
    },
}

/// Configuration spaces.
///
/// These are the possible configuration spaces defined in the USB4 specification. If the
/// configuration space is not known, it is set to [`ConfigSpace::Unknown`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum ConfigSpace {
    /// Config space is not known.
    Unknown,
    /// Adapter path config space.
    Path,
    /// Adapter config space.
    Adapter,
    /// Router config space.
    Router,
    /// Adapter counters config space.
    Counters,
}

impl From<u8> for ConfigSpace {
    fn from(cs: u8) -> Self {
        match cs {
            0 => Self::Path,
            1 => Self::Adapter,
            2 => Self::Router,
            3 => Self::Counters,
            _ => panic!("unknown config space"),
        }
    }
}

impl Display for ConfigSpace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            Self::Path => "Path",
            Self::Adapter => "Adapter",
            Self::Router => "Router",
            Self::Counters => "Counters",
            _ => panic!("unknown config space"),
        };
        write!(f, "{s}")
    }
}

/// Protocol Defined Field.
///
/// These map directly to the USB4 specification. The protocol here refers to the control packets
/// defined in the appendix B. "Summary of Transport Layer Packets". The kernel driver also supports
/// firmware connection manager (ICM) specific packets. These are prefixed with `Icm` and not found
/// in the USB4 specification.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pdf {
    /// Control packet type is not known.
    Unknown,
    /// Read Request.
    ReadRequest,
    /// Read Response.
    ReadResponse,
    /// Write Request.
    WriteRequest,
    /// Write Response.
    WriteResponse,
    /// Notification Packet.
    Notification,
    /// Notification Acknowledgement Packet.
    NotificationAck,
    /// Hot Plug Event Packet.
    HotPlugEvent,
    /// Inter-Domain Request.
    XdomainRequest,
    /// Inter-Domain Response.
    XdomainResponse,
    /// Enhanced Notification Acknowledgement Packet.
    EnhancedNotificationAck,
    /// Notification from ICM.
    IcmEvent,
    /// Request to ICM.
    IcmRequest,
    /// Response from ICM.
    IcmResponse,
}

impl Pdf {
    /// Returns USB4 spec `PDF` number.
    pub fn to_num(&self) -> Option<u32> {
        match *self {
            Self::ReadRequest | Self::ReadResponse => Some(1),
            Self::WriteRequest | Self::WriteResponse => Some(2),
            Self::Notification => Some(3),
            Self::NotificationAck => Some(4),
            Self::HotPlugEvent => Some(5),
            Self::XdomainRequest => Some(6),
            Self::XdomainResponse => Some(7),
            Self::EnhancedNotificationAck => Some(8),
            Self::IcmEvent => Some(10),
            Self::IcmRequest => Some(11),
            Self::IcmResponse => Some(12),
            _ => None,
        }
    }
}

impl Display for Pdf {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            Self::ReadRequest => "Read Request",
            Self::ReadResponse => "Read Response",
            Self::WriteRequest => "Write Request",
            Self::WriteResponse => "Write Response",
            Self::Notification => "Notification Packet",
            Self::NotificationAck => "Notification Acknowledgement Packet",
            Self::HotPlugEvent => "Hot Plug Event Packet",
            Self::XdomainRequest => "Inter-Domain Request",
            Self::XdomainResponse => "Inter-Domain Response",
            Self::EnhancedNotificationAck => "Enhanced Notification Acknowledgement Packet",
            Self::IcmEvent => "ICM Event",
            Self::IcmRequest => "ICM Request",
            Self::IcmResponse => "ICM Response",
            _ => "Unknown",
        };
        write!(f, "{s}")
    }
}

impl Serialize for Pdf {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Finds devices with given address.
///
/// Find devices on the bus. If `address` is passed then finds all matching that, otherwise returns
/// all devices on the bus.
///
/// # Examples
/// Find and print all devices on the bus.
/// ```no_run
/// # use std::io;
/// # fn main() -> io::Result<()> {
/// for device in tbtools::find_devices(None)? {
///     println!("device: {:?}", device);
/// }
/// # Ok(())
/// # }
/// ```
/// Find host router of the first domain. You can use [`find_device()`](find_device()) here as
/// well.
/// ```no_run
/// # use std::io;
/// use tbtools::Address;
///
/// # fn main() -> io::Result<()> {
/// let address = Address::Router { domain: 0, route: 0 };
/// let devices = tbtools::find_devices(Some(&address))?;
/// assert_eq!(devices.len(), 1);
/// # Ok(())
/// # }
/// ```
pub fn find_devices(address: Option<&Address>) -> io::Result<Vec<Device>> {
    let mut enumerator = udev::Enumerator::new()?;

    enumerator.match_subsystem("thunderbolt")?;

    match address {
        Some(Address::Domain { domain }) => {
            enumerator.match_sysname(format!("domain{domain}"))?;
        }
        Some(Address::Router { domain, route }) => {
            enumerator.match_property("DEVTYPE", Kind::Router.to_string())?;
            enumerator.match_sysname(format!("{domain}-{route:x}"))?;
        }
        Some(Address::Xdomain { domain, route }) => {
            enumerator.match_property("DEVTYPE", Kind::Xdomain.to_string())?;
            enumerator.match_sysname(format!("{domain}-{route:x}"))?;
        }
        Some(Address::Retimer {
            domain,
            route,
            adapter,
            index,
        }) => {
            enumerator.match_sysname(format!("{domain}-{route:x}:{adapter}.{index}"))?;
        }
        _ => (),
    }

    let mut devices = Vec::new();

    for udev in enumerator.scan_devices()? {
        if let Some(device) = Device::parse(udev) {
            devices.push(device);
        }
    }

    devices.sort();

    Ok(devices)
}

/// Find a given Thunderbolt device.
///
/// Finds a single device with matching `address` on the bus.
/// # Examples
/// Find host router of the first domain.
/// ```no_run
/// # use std::io;
/// use tbtools::Address;
///
/// # fn main() -> io::Result<()> {
/// let address = Address::Router {domain: 0, route: 0 };
/// let device = tbtools::find_device(&address)?;
/// assert!(device.is_some());
/// # Ok(())
/// # }
/// ```
pub fn find_device(address: &Address) -> io::Result<Option<Device>> {
    Ok(find_devices(Some(address))?.pop())
}

fn is_authorized(device: &Device) -> bool {
    device.authorized().unwrap_or(false)
}

/// Authorize or de-authorize given Thunderbolt device.
///
/// Authorizes PCIe tunnel of a given device and all the parent devices as well. Only supports
/// [`User`](SecurityLevel::User) authorization for parent devices. For `device` also
/// [`Secure`](SecurityLevel::Secure) is supported but the key must be set in advance by calling
/// [`set_key()`](Device::set_key()).
/// # Examples
/// Authorize a device with route `1`.
/// ```no_run
/// # use std::io;
/// use tbtools::Address;
///
/// # fn main() -> io::Result<()> {
/// let address = Address::Router { domain: 0, route: 1 };
/// if let Some(mut device) = tbtools::find_device(&address)? {
///     tbtools::authorize_device(&mut device, 1)?;
/// }
/// # Ok(())
/// # }
/// ```
pub fn authorize_device(device: &mut Device, authorized: u32) -> io::Result<()> {
    if !device.is_router() {
        return Err(Error::from(ErrorKind::InvalidData));
    }
    if authorized == 0 && !is_authorized(device) {
        return Ok(());
    } else if authorized == 1 {
        if is_authorized(device) {
            return Ok(());
        }
        if let Some(mut parent) = device.parent() {
            if !parent.authorized().unwrap_or(false) {
                authorize_device(&mut parent, authorized)?;
            }
        }
    }
    device.authorize(authorized)
}
