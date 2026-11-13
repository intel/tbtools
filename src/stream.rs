// USB4STREAM helper functions
//
// Copyright (C) 2026, Intel Corporation
// Author: Alan Borzeszkowski <alan.borzeszkowski@linux.intel.com>

use crate::{Device, device::SERVICE_RE, find_devices, util};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const HOPID_MIN: i32 = 8;
pub const HOPID_AUTO_ALLOC: i32 = -1;

const CONFIGFS_ROOT: &str = "/sys/kernel/config";
const CONFIGFS_PATH: &str = "/sys/kernel/config/thunderbolt/stream";
const DEFAULT_NAME: &str = "auto";
const DEFAULT_NAME_MAX_INDEX: u32 = 10;

/// USB4STREAM service group
///
/// Stream devices of a USB4STREAM group are configured through configfs.
pub struct Stream {
    device: Option<Device>,
    name: String,
}

impl Stream {
    fn new(device: Device) -> Self {
        Self {
            name: device.kernel_name(),
            device: Some(device),
        }
    }

    fn from_configfs(name: String) -> Self {
        Self { name, device: None }
    }

    /// Mounts configfs if not already mounted
    pub fn mount_configfs() -> io::Result<()> {
        util::mount_fs("configfs", CONFIGFS_ROOT)
    }

    /// Returns `true` if kernel supports USB4STREAM
    ///
    /// This requires configfs to be mounted and the 'thunderbolt_stream'
    /// module to be loaded.
    pub fn is_supported() -> bool {
        Path::new(CONFIGFS_PATH).is_dir()
    }

    /// Returns all Streams sorted by their address
    ///
    /// Connected Streams come first. They are followed by Streams
    /// that exist only in configfs (peer machine not connected).
    pub fn find_all() -> io::Result<Vec<Self>> {
        let mut streams: Vec<Self> = find_devices(None)?
            .into_iter()
            .filter(|d| d.is_service() && d.service_key().as_deref() == Some("stream"))
            .map(Self::new)
            .collect();

        // Also find Stream groups that are not connected to peer machine
        let mut not_connected: Vec<Self> = Self::find_configfs_groups()?
            .into_iter()
            .filter(|name| !streams.iter().any(|s| s.kernel_name() == name))
            .map(Self::from_configfs)
            .collect();

        not_connected.sort_by(|a, b| a.name.cmp(&b.name));
        streams.append(&mut not_connected);

        Ok(streams)
    }

    fn find_configfs_groups() -> io::Result<Vec<String>> {
        let entries = match fs::read_dir(CONFIGFS_PATH) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };

        Ok(entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|ft| ft.is_dir()))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| SERVICE_RE.is_match(name))
            .collect())
    }

    /// Returns kernel name of Stream (e.g., 1-1.0)
    pub fn kernel_name(&self) -> &str {
        &self.name
    }

    /// Returns the bus device of the Stream, 'None' if peer is not connected
    pub fn kernel_device(&self) -> Option<&Device> {
        self.device.as_ref()
    }

    /// Returns XDomain this Stream is connected through, `None` if the
    /// peer is not connected.
    pub fn peer(&self) -> Option<Device> {
        self.device.as_ref()?.parent().filter(|d| d.is_xdomain())
    }

    /// Returns `true` if this Stream is actively connected to another
    /// USB4STREAM host
    pub fn is_connected(&self) -> bool {
        self.device.is_some()
    }

    fn path(&self) -> PathBuf {
        Path::new(CONFIGFS_PATH).join(&self.name)
    }

    /// Returns `true` if Stream exists in configfs
    pub fn group_exists(&self) -> bool {
        self.path().is_dir()
    }

    /// Creates Stream group directory in configfs
    pub fn add(&self) -> io::Result<()> {
        fs::create_dir(self.path())
    }

    /// Removes Stream group in configfs along with its devices
    pub fn remove(&self) -> io::Result<()> {
        // Remove Stream devices first
        for sdev in self.devices()? {
            sdev.remove()?;
        }

        // Then whole Stream group
        fs::remove_dir(self.path())
    }

    /// Returns stream devices of the Stream, sorted by index
    pub fn devices(&self) -> io::Result<Vec<StreamDevice>> {
        let mut stream_devices: Vec<StreamDevice> = fs::read_dir(self.path())?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|ft| ft.is_dir()))
            .map(|entry| StreamDevice::get(entry.path()))
            .collect::<io::Result<_>>()?;

        stream_devices.sort_by_key(|sd| sd.index);
        Ok(stream_devices)
    }

    fn device_path(&self, name: &str) -> io::Result<PathBuf> {
        if Path::new(name).file_name() != Some(OsStr::new(name)) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("Invalid stream device name '{name}'"),
            ));
        }

        Ok(self.path().join(name))
    }

    /// Return Stream device by name
    pub fn device(&self, name: &str) -> io::Result<StreamDevice> {
        StreamDevice::get(self.device_path(name)?)
    }

    /// Returns `true` if Stream device with this name exists in configfs
    pub fn device_exists(&self, name: &str) -> bool {
        self.device_path(name).is_ok_and(|path| path.is_dir())
    }

    /// Adds Stream device to Stream group
    ///
    /// Stream device name is optional. If not specified, default name will be
    /// used, that is: autoN, where N={0..9}.
    pub fn device_add(&self, name: Option<&str>) -> io::Result<StreamDevice> {
        let name = match name {
            Some(name) => name.to_string(),
            None => self.device_set_default_name().ok_or_else(|| {
                io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("No free stream device name in Stream {}", self.name),
                )
            })?,
        };

        let path = self.device_path(&name)?;
        fs::create_dir(&path)?;

        StreamDevice::get(path.clone()).map_err(|e| match fs::remove_dir(&path) {
            Ok(()) => e,
            Err(rm_err) => io::Error::new(
                e.kind(),
                format!("{e}: failed to remove {}: {rm_err}", path.display()),
            ),
        })
    }

    fn device_set_default_name(&self) -> Option<String> {
        (0..DEFAULT_NAME_MAX_INDEX)
            .map(|index| format!("{DEFAULT_NAME}{index}"))
            .find(|name| !self.device_exists(name))
    }
}

/// Pairing status of USB4STREAM peer host
pub enum Pairing {
    /// Peer host already configured stream device for us
    Paired,
    /// HopIDs allocated locally, pending same operation on peer host
    Pending,
}

/// Stream device of a Stream group
pub struct StreamDevice {
    path: PathBuf,
    name: String,
    index: u32,
    in_hopid: i32,
    out_hopid: i32,
}

impl StreamDevice {
    fn get(path: PathBuf) -> io::Result<Self> {
        let name = path
            .file_name()
            .ok_or_else(|| {
                io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("Invalid stream device path {}", path.display()),
                )
            })?
            .to_string_lossy()
            .into_owned();

        Ok(Self {
            index: read_attribute(&path, "index")?,
            in_hopid: read_attribute(&path, "in_hopid")?,
            out_hopid: read_attribute(&path, "out_hopid")?,
            name,
            path,
        })
    }

    /// Returns name of Stream device
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns IN HopID of Stream device
    pub fn in_hopid(&self) -> i32 {
        self.in_hopid
    }

    /// Returns OUT HopID of Stream device
    pub fn out_hopid(&self) -> i32 {
        self.out_hopid
    }

    /// Returns kernel device node of Stream device
    pub fn devnode(&self) -> String {
        format!("/dev/tbstream{}", self.index)
    }

    /// Returns `true` if both HopIDs are allocated
    pub fn is_configured(&self) -> bool {
        self.in_hopid >= HOPID_MIN && self.out_hopid >= HOPID_MIN
    }

    /// Auto allocate HopIDs for this Stream device
    pub fn configure(&self) -> io::Result<Pairing> {
        if self.is_configured() {
            return Ok(Pairing::Paired);
        }

        for attr in ["in_hopid", "out_hopid"] {
            write_attribute(&self.path, attr, HOPID_AUTO_ALLOC)?;
        }

        Ok(Pairing::Pending)
    }

    /// Clear HopIDs of stream device and remove it from configfs
    pub fn remove(self) -> io::Result<()> {
        for attr in ["in_hopid", "out_hopid"] {
            write_attribute(&self.path, attr, 0)?;
        }

        fs::remove_dir(&self.path)
    }
}

fn read_attribute<T>(path: &Path, attr: &str) -> io::Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    let content = fs::read_to_string(path.join(attr))?;

    content.trim().parse().map_err(|e| {
        io::Error::new(
            ErrorKind::InvalidData,
            format!("Failed to parse {attr}: {e}"),
        )
    })
}

fn write_attribute<T>(path: &Path, attr: &str, value: T) -> io::Result<()>
where
    T: fmt::Display,
{
    fs::write(path.join(attr), value.to_string())
        .map_err(|e| io::Error::new(e.kind(), format!("Failed to write {attr}: {e}")))
}
