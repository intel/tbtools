// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Implements register read and write support through kernel `debugfs` interface.
//!
//! For certain functionality, such as writing of the registers, to be available the kernel must be
//! compiled with `CONFIG_USB4_DEBUGFS_WRITE=y` and possibly with
//! `CONFIG_USB4_DEBUGFS_MARGINING=y`.
//!
//! Calling [`Device`]'s [`registers_writable()`](Device::registers_writable()) can be used to determine whether registers can be
//! written to.

use crate::{device::Device, genmask, usb4, util};
use lazy_static::lazy_static;
use nix::{errno::Errno, mount};
use num_traits::Num;
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Result, Write},
    ops::RangeInclusive,
    path::PathBuf,
};

lazy_static! {
    // Pull in the register descriptions
    static ref NAMES: Value = serde_json::from_str(
        include_str!("data/registers.json")
    )
    .unwrap();
}

const DEBUGFS_ROOT: &str = "/sys/kernel/debug/thunderbolt";
const DEBUGFS_REGS: &str = "regs";
const DEBUGFS_PATH: &str = "path";
const DEBUGFS_COUNTERS: &str = "counters";

const DEBUGFS_HELP: &str = "Note debugfs may not be mounted. To do that manually
you can run following command as root:

  mount -t debugfs none /sys/kernel/debug
";

/// Type of an adapter.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Eq, Hash, Ord)]
pub enum Type {
    /// Unknown adapter.
    Unknown,
    /// Not implemented adapter.
    Inactive,
    /// Lane adapter.
    Lane,
    /// Host interface adapter.
    HostInterface,
    /// DisplayPort IN adapter.
    DisplayPortIn,
    /// DisplayPort OUT adapter.
    DisplayPortOut,
    /// PCIe downstream adapter.
    PcieDown,
    /// PCIe upstream adapter.
    PcieUp,
    /// USB 3.x (Gen X) downstream adapter.
    Usb3Down,
    /// USB 3.x (Gen X) upstream adapter.
    Usb3Up,
}

impl From<&str> for Type {
    fn from(s: &str) -> Self {
        match s {
            "Inactive" => Self::Inactive,
            "Lane" => Self::Lane,
            "Host Interface" => Self::HostInterface,
            "DisplayPort IN" => Self::DisplayPortIn,
            "DisplayPort OUT" => Self::DisplayPortOut,
            "PCIe Down" => Self::PcieDown,
            "PCIe Up" => Self::PcieUp,
            "USB 3 Down" => Self::Usb3Down,
            "USB 3 Up" => Self::Usb3Up,
            _ => Self::Unknown,
        }
    }
}

impl Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            Self::Inactive => "Inactive",
            Self::Lane => "Lane",
            Self::HostInterface => "Host Interface",
            Self::DisplayPortIn => "DisplayPort IN",
            Self::DisplayPortOut => "DisplayPort OUT",
            Self::PcieDown => "PCIe Down",
            Self::PcieUp => "PCIe Up",
            Self::Usb3Down => "USB 3 Down",
            Self::Usb3Up => "USB 3 Up",
            _ => "Unknown",
        };
        write!(f, "{}", s)
    }
}

/// Field metadata description.
#[derive(Clone, Debug)]
pub struct BitField {
    range: RangeInclusive<u8>,
    name: String,
    short_name: Option<String>,
    value_names: Option<HashMap<u32, String>>,
}

impl BitField {
    pub fn new(
        range: RangeInclusive<u8>,
        name: &str,
        short_name: Option<&str>,
        value_names: Option<HashMap<u32, String>>,
    ) -> Self {
        Self {
            range,
            name: String::from(name),
            short_name: short_name.map(String::from),
            value_names,
        }
    }

    /// Returns start and end bit range of the field (inclusive).
    pub fn range(&self) -> &RangeInclusive<u8> {
        &self.range
    }

    /// Returns long name of the field extracted from the USB4 spec.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns short name of the field from the USB4 spec.
    pub fn short_name(&self) -> Option<&str> {
        self.short_name.as_deref()
    }

    /// Returns name for the value if it is known.
    pub fn value_name(&self, value: u32) -> Option<&str> {
        self.value_names.as_ref()?.get(&value).map(|s| s.as_str())
    }

    fn parse_enum(value: &Value) -> Option<HashMap<u32, String>> {
        let values = value.get("values")?.as_array()?;

        let mut map = HashMap::new();

        for v in values {
            let value = v.get("value")?.as_u64()? as u32;
            let name = v.get("name")?.as_str()?;

            map.insert(value, name.to_string());
        }

        Some(map)
    }

    fn parse_one(value: &Value) -> Option<BitField> {
        let start_bit = value.get("start_bit")?.as_u64().unwrap() as u8;
        let end_bit = value.get("end_bit")?.as_u64().unwrap() as u8;
        let name = value.get("name")?.as_str().unwrap();
        let value_names = Self::parse_enum(value);

        if start_bit > end_bit {
            eprintln!(
                "Warning: invalid range {} > {} in {}",
                start_bit, end_bit, name
            );
            return None;
        }

        let short_name = value.get("short_name").map(|c| c.as_str().unwrap());

        Some(BitField::new(
            start_bit..=end_bit,
            name,
            short_name,
            value_names,
        ))
    }

    pub(crate) fn parse(value: &Value) -> Option<Vec<BitField>> {
        if let Some(fields) = value.get("bitfields")?.as_array() {
            let mut all_fields = Vec::new();

            for field in fields {
                if let Some(field) = Self::parse_one(field) {
                    all_fields.push(field);
                }
            }

            return Some(all_fields);
        }

        None
    }
}

/// Register metadata description. These are parsed from `registers.json`.
#[derive(Clone, Debug)]
struct Metadata {
    /// Maps to field `cap_id` in `registers.json` or `None` if not present.
    cap_id: Option<u16>,
    /// Maps to field `vs_cap_id` in `registers.json` or `None` if not present.
    vs_cap_id: Option<u16>,
    /// Adapter types that this metadate applies to or `None` if applies to all adapters.
    adapter_types: Option<Vec<Type>>,
    /// USB4 spec name for the register.
    name: String,
    /// Separate fields of the register.
    fields: Option<Vec<BitField>>,
}

impl Metadata {
    /// Returns `true` if the `adapter_type` is included in this metadata. Also if there is no
    /// `adapter_type` it is treated as match.
    fn match_type(&self, adapter_type: Type) -> bool {
        if let Some(ref adapter_types) = self.adapter_types {
            return adapter_types.iter().any(|t| *t == adapter_type);
        }
        true
    }

    fn parse(value: &Value) -> Option<Self> {
        let name = String::from(value.get("name")?.as_str().unwrap());

        let cap_id = value.get("cap_id").map(|c| c.as_u64().unwrap() as u16);
        let vs_cap_id = value.get("vs_cap_id").map(|c| c.as_u64().unwrap() as u16);

        let adapter_types = if let Some(adapter_type) = value.get("adapter_type") {
            let mut adapter_types: Vec<Type> = Vec::new();
            let adapter_type = adapter_type.as_array().unwrap();
            for at in adapter_type {
                adapter_types.push(at.as_str().unwrap().into());
            }
            Some(adapter_types)
        } else {
            None
        };

        let fields = BitField::parse(value);

        Some(Self {
            cap_id,
            vs_cap_id,
            adapter_types,
            name,
            fields,
        })
    }

    fn lookup_from_json(
        value: &Value,
        offset: u16,
        adapter_type: Option<Type>,
    ) -> Option<Vec<Self>> {
        let mut metadata = Vec::new();

        for desc in value.as_array().unwrap() {
            // Offset is always relative to the capability.
            let desc_offset = desc.get("offset").unwrap().as_u64().unwrap() as u16;

            if offset == desc_offset {
                let md = Self::parse(desc)?;

                // Match with the type if it is given.
                if let Some(adapter_type) = adapter_type {
                    if md.match_type(adapter_type) {
                        metadata.push(md);
                    }
                } else {
                    metadata.push(md);
                }
            }
        }

        if metadata.is_empty() {
            None
        } else {
            Some(metadata)
        }
    }

    fn from_router_offset(offset: u16) -> Option<Vec<Self>> {
        Self::lookup_from_json(NAMES.get("router").unwrap(), offset, None)
    }

    fn from_adapter_type_and_offset(
        name: &str,
        adapter_type: Type,
        offset: u16,
    ) -> Option<Vec<Self>> {
        Self::lookup_from_json(NAMES.get(name).unwrap(), offset, Some(adapter_type))
    }
}

/// [`Register`] or similar with optional name attached.
pub trait Name {
    /// Returns the name from metadata if known.
    fn name(&self) -> Option<&str>;
}

fn match_name(field: &BitField, name: &str) -> bool {
    let name = name.to_lowercase();

    if field.name.to_lowercase() == name {
        return true;
    }
    if let Some(short_name) = &field.short_name {
        if short_name.to_lowercase() == name {
            return true;
        }
    }
    false
}

/// Value that can be split into bit fields.
pub trait BitFields<T: PartialOrd + Num> {
    /// Returns the field metadata if known.
    fn fields(&self) -> Option<&Vec<BitField>>;

    /// Returns field value.
    fn field_value(&self, field: &BitField) -> T;

    /// Returns field metadata by field name or short name. The match is case insensitive.
    fn field_by_name(&self, name: &str) -> Option<&BitField> {
        self.fields().as_ref()?.iter().find(|f| match_name(f, name))
    }

    /// Returns `true` if field with given name exists.
    fn has_field(&self, name: &str) -> bool {
        self.field_by_name(name).is_some()
    }

    /// Returns field value by name or short name.
    ///
    /// # Panics
    /// Panics if field with given name (or short name) does not exist.
    fn field(&self, name: &str) -> T {
        if let Some(field) = self.field_by_name(name) {
            return self.field_value(field);
        }
        panic!("BitField {} does not exist\n", name);
    }

    /// Returns field value as bit flag by name or short name.
    ///
    /// # Panics
    /// Panics if field with given name (or short name) does not exist, or it is not bit field.
    /// # Examples
    /// ```no_run
    /// # use std::io;
    /// # use tbtools::{debugfs::BitFields, Address};
    /// # fn main() -> io::Result<()> {
    /// # if let Some(device) = tbtools::find_device(&Address::Router { domain: 0, route: 0 })? {
    /// let reg = device.register_by_name("TMU_RTR_CS_0").unwrap();
    /// // Expect the router TMU supports uni-directional time-sync.
    /// assert!(reg.flag("UCAP"));
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    fn flag(&self, name: &str) -> bool {
        if let Some(field) = self.field_by_name(name) {
            if field.range.len() != 1 {
                panic!("BitField {} is not bit flag\n", name);
            }
            return !self.field_value(field).is_zero();
        }
        panic!("BitField {} does not exist\n", name);
    }
}

/// Config space register value.
#[derive(Clone, Debug)]
pub struct Register {
    /// Register absolute offset.
    offset: u32,
    /// Relative offset inside capability.
    relative_offset: u16,
    /// Register capability ID.
    cap_id: u16,
    /// Vendor specific capability ID. Only present in router and adapter config space registers.
    vs_cap_id: u16,
    /// Register value.
    value: u32,
    /// Is this register changed.
    changed: bool,
    /// Metadata for the register if available.
    metadata: Option<Metadata>,
}

impl Register {
    fn new(offset: u32, relative_offset: u16, cap_id: u16, vs_cap_id: u16, value: u32) -> Self {
        Self {
            offset,
            relative_offset,
            cap_id,
            vs_cap_id,
            value,
            changed: false,
            metadata: None,
        }
    }

    fn set_metadata(&mut self, metadata: Vec<Metadata>) {
        let mut matches: Vec<_> = metadata
            .into_iter()
            .filter(|d| {
                // Match `cap_id` if it is non-zero.
                if self.cap_id != 0 {
                    if let Some(cap_id) = d.cap_id {
                        return self.cap_id == cap_id;
                    } else {
                        return false;
                    }
                } else if d.cap_id.is_none() && d.vs_cap_id.is_none() {
                    return true;
                }
                false
            })
            .filter(|d| {
                // Match `vs_cap_id` if it is non-zero.
                if self.vs_cap_id != 0 {
                    if let Some(vs_cap_id) = d.vs_cap_id {
                        return self.vs_cap_id == vs_cap_id;
                    } else {
                        return false;
                    }
                }
                true
            })
            .collect();

        // There needs to be unique match.
        if matches.len() == 1 {
            self.metadata = Some(matches.remove(0));
        }
    }

    fn parse_debugfs(regs: &str) -> Option<Self> {
        if regs.starts_with('#') {
            return None;
        }

        let values: Vec<&str> = regs.split_ascii_whitespace().collect();
        if values.len() < 4 {
            return None;
        }

        let offset = util::parse_hex::<u32>(values[0])?;
        let relative_offset = values[1].parse::<u16>().ok()?;
        let cap_id: u16;
        let vs_cap_id: u16;
        let value: u32;

        if values.len() == 4 {
            // Path and Counters, no caps.
            cap_id = 0;
            vs_cap_id = 0;
            value = util::parse_hex::<u32>(values[3])?;
        } else {
            cap_id = util::parse_hex::<u16>(values[2])?;
            vs_cap_id = util::parse_hex::<u16>(values[3])?;
            value = util::parse_hex::<u32>(values[4])?;
        }

        Some(Self::new(offset, relative_offset, cap_id, vs_cap_id, value))
    }

    /// Returns register absolute offset in the config space.
    pub fn offset(&self) -> u32 {
        // XXX: use u16 for the type
        self.offset
    }

    /// Returns capability relative offset.
    pub fn relative_offset(&self) -> u16 {
        self.relative_offset
    }

    /// Returns capability ID.
    pub fn cap_id(&self) -> u16 {
        self.cap_id
    }

    /// Returns vendor specific capability (VSEC) ID.
    pub fn vs_cap_id(&self) -> u16 {
        self.vs_cap_id
    }

    /// Returns current register value.
    pub fn value(&self) -> u32 {
        self.value
    }

    /// Sets the current register value.
    pub fn set_value(&mut self, value: u32) {
        if self.value != value {
            self.value = value;
            self.changed = true;
        }
    }

    /// Sets field value.
    ///
    /// # Panics
    /// Panics if field with given name (or short name) does not exist.
    pub fn set_field(&mut self, name: &str, value: u32) {
        if let Some(field) = self.field_by_name(name) {
            let mask = genmask!(*field.range.end() as u32, *field.range.start() as u32);
            let shift = *field.range.start();

            self.value &= !mask;
            self.value |= value << shift;

            self.changed = true;
            return;
        }
        panic!("Bit Field {} does not exit\n", name);
    }

    /// Returns true if the register value has been changed.
    pub fn is_changed(&self) -> bool {
        self.changed
    }
}

impl Name for Register {
    /// If the register name is known it is returned here.
    ///
    /// All the USB4 spec registers are known currently. These can be added by modifying the
    /// `registers.json`.
    fn name(&self) -> Option<&str> {
        Some(&self.metadata.as_ref()?.name)
    }
}

impl BitFields<u32> for Register {
    /// Returns the field metadata for this register if known.
    fn fields(&self) -> Option<&Vec<BitField>> {
        self.metadata.as_ref()?.fields.as_ref()
    }

    fn field_value(&self, field: &BitField) -> u32 {
        let mask = genmask!(*field.range.end() as u32, *field.range.start() as u32);
        let shift = *field.range.start();

        (self.value & mask) >> shift
    }
}

/// Returns debugfs root as PathBuf
pub(crate) fn path_buf() -> Result<PathBuf> {
    let path_buf = PathBuf::from(DEBUGFS_ROOT);

    if !path_buf.exists() {
        eprintln!("{}", DEBUGFS_HELP);
        return Err(Error::from(ErrorKind::NotFound));
    }

    Ok(path_buf)
}

fn router_path_buf(router: &Device) -> Result<PathBuf> {
    let mut path_buf = path_buf()?;
    path_buf.push(router.kernel_name());
    path_buf.push(DEBUGFS_REGS);
    Ok(path_buf)
}

/// Mounts debugfs if not already mounted. User must be `root`.
pub fn mount() -> Result<()> {
    match mount::mount(
        None::<&PathBuf>,
        &PathBuf::from("/sys/kernel/debug"),
        Some("debugfs"),
        mount::MsFlags::empty(),
        None::<&PathBuf>,
    ) {
        // OK if already mounted.
        Err(Errno::EBUSY) => Ok(()),
        Err(err) => Err(err.into()),
        Ok(_) => Ok(()),
    }
}

fn read(path_buf: &PathBuf, offset: Option<u32>, nregs: Option<usize>) -> Result<Vec<Register>> {
    let file = File::open(path_buf)?;
    let reader = BufReader::new(file);
    let offset = offset.unwrap_or(0);
    let mut regs = Vec::new();

    // Initially we just read all the registers as that's pretty much the same anyway.
    for line in reader.lines() {
        if let Some(reg) = Register::parse_debugfs(&line?) {
            regs.push(reg);
        }
    }

    // Then filter out what was asked.
    let num = regs.len();

    let mut regs: Vec<_> = regs
        .into_iter()
        .filter(|r| r.offset >= offset)
        .enumerate()
        .filter(|(i, _)| *i < nregs.unwrap_or(num))
        .map(|(_, r)| r)
        .collect();

    // Kernel reports them by capability so sort them out here by the actual offset.
    regs.sort_by(|a, b| a.offset.cmp(&b.offset));

    Ok(regs)
}

fn write_changed(path_buf: &PathBuf, regs: &[Register]) -> Result<()> {
    let file = OpenOptions::new().write(true).open(path_buf)?;
    let mut writer = BufWriter::new(file);

    for reg in regs.iter().filter(|r| r.is_changed()) {
        writeln!(&mut writer, "0x{:04x} 0x{:08x}", reg.offset, reg.value)?;
    }

    writer.flush()?;

    Ok(())
}

/// Parsed enabled path configuration space entry.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Path {
    /// Input adapter number
    in_adapter: u16,
    /// Input HopID
    in_hop: u16,
    /// Output adapter number
    out_adapter: u16,
    /// Output HopID
    out_hop: u16,
    /// PM packet support bit
    pmps: bool,
}

impl Path {
    /// Creates a new path entry from `PATH_CS_0` register.
    fn from(in_adapter: u16, in_hop: u16, path_cs_0: u32) -> Option<Self> {
        if (path_cs_0 & usb4::PATH_CS_0_VALID) == usb4::PATH_CS_0_VALID {
            let out_hop = path_cs_0 & usb4::PATH_CS_0_OUT_HOP_MASK;
            let out_adapter =
                (path_cs_0 & usb4::PATH_CS_0_OUT_ADAPTER_MASK) >> usb4::PATH_CS_0_OUT_ADAPTER_SHIFT;
            let pmps = (path_cs_0 & usb4::PATH_CS_0_PMPS) > 0;

            return Some(Self {
                in_adapter,
                in_hop,
                out_adapter: out_adapter as u16,
                out_hop: out_hop as u16,
                pmps,
            });
        }
        None
    }

    /// Returns in adapter number.
    pub fn in_adapter(&self) -> u16 {
        self.in_adapter
    }

    /// Returns in `HopID`.
    pub fn in_hop(&self) -> u16 {
        self.in_hop
    }

    /// Returns out adapter number.
    pub fn out_adapter(&self) -> u16 {
        self.out_adapter
    }

    /// Returns out `HopID`.
    pub fn out_hop(&self) -> u16 {
        self.out_hop
    }

    /// Returns `true` if PM package support bit is set for this entry.
    pub fn pmps(&self) -> bool {
        self.pmps
    }
}

/// Adapter state
///
/// These are all possible states the adapter can be. Not all states apply to all adapter types.
/// For instance CL states only apply to lane adapters.
#[derive(Copy, Clone, Debug, PartialEq, PartialOrd, Eq, Hash, Ord)]
pub enum State {
    /// Adapter state is not known.
    Unknown,
    /// Protocol adapter is disabled.
    Disabled,
    /// Protocol adapter is enabled.
    Enabled,
    /// Lane adapter is training.
    Training,
    /// Lane adapter is in CL0 (active).
    Cl0,
    /// Lane adapter is in CL0s (standby).
    Cl0sTx,
    /// Lane adapter is in CL0s (standby).
    Cl0sRx,
    /// Lane adapter is in CL1 power state.
    Cl1,
    /// Lane adapter is in CL2 power state.
    Cl2,
    /// Lane is disabled.
    Cld,
}

/// Adapter of a router.
///
/// This represents a single adapter of a router. When an adapter is created it contains its
/// adapter config space registers. All other config spaces (path or counters) need to be read
/// separately. The way to do this (and also if there is need to refresh adapter config space) is
/// shown below.
///
/// # Examples
/// Access adapter and path registers of device adapter number `1`.
/// ```no_run
/// # use std::io;
/// # use tbtools::{debugfs::{BitFields, Adapter}, Address};
/// # fn main() -> io::Result<()> {
/// # if let Some(mut device) = tbtools::find_device(&Address::Router { domain: 0, route: 0 })? {
/// // Must be called before accessing adapter registers.
/// device.read_adapters()?;
///
/// if let Some(mut adapter) = device.adapter_mut(1) {
///     // Adapter config space is already read. Let's read path config space.
///     if let Ok(_) = adapter.read_paths() {
///         // Read path config space with HopID 8.
///         if let Some(path) = adapter.path(8) {
///             assert_eq!(path.in_hop(), 8);
///         }
///     }
///
///     // Read ADP_CS_3, must be present so we can call unrap() directly.
///     let cs_3 = adapter.register_by_name("ADP_CS_3").unwrap();
///     assert_eq!(cs_3.field("Adapter Number"), 1);
/// }
/// # }
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Adapter {
    adapter: u16,
    kind: Type,
    state: State,
    debugfs_path: PathBuf,
    regs: Option<Vec<Register>>,
    path_regs: Option<Vec<Register>>,
    paths: Option<Vec<Path>>,
    counter_regs: Option<Vec<Register>>,
    usb4: bool,
    upstream: bool,
}

impl Adapter {
    fn new(adapter: u16, debugfs_path: PathBuf, usb4: bool, upstream: bool) -> Self {
        Self {
            adapter,
            kind: Type::Inactive,
            state: State::Unknown,
            debugfs_path,
            regs: None,
            path_regs: None,
            paths: None,
            counter_regs: None,
            usb4,
            upstream,
        }
    }

    fn parse_kind(regs: &[Register]) -> Type {
        let val = regs[usb4::ADP_CS_2].value & usb4::ADP_CS_2_TYPE_MASK;
        match val {
            usb4::ADP_CS_2_TYPE_INACTIVE => Type::Inactive,
            usb4::ADP_CS_2_TYPE_LANE => Type::Lane,
            usb4::ADP_CS_2_TYPE_NHI => Type::HostInterface,
            usb4::ADP_CS_2_TYPE_DP_IN => Type::DisplayPortIn,
            usb4::ADP_CS_2_TYPE_DP_OUT => Type::DisplayPortOut,
            usb4::ADP_CS_2_TYPE_PCIE_DOWN => Type::PcieDown,
            usb4::ADP_CS_2_TYPE_PCIE_UP => Type::PcieUp,
            usb4::ADP_CS_2_TYPE_USB3_DOWN => Type::Usb3Down,
            usb4::ADP_CS_2_TYPE_USB3_UP => Type::Usb3Up,
            _ => Type::Unknown,
        }
    }

    fn parse_state(&self) -> State {
        match self.kind {
            Type::Lane => {
                if let Some(reg) = self.register_by_name("LANE_ADP_CS_1") {
                    let state = reg.field("Adapter State");

                    match state {
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_DISABLED => State::Disabled,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_TRAINING => State::Training,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CL0 => State::Cl0,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CL0S_TX => State::Cl0sTx,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CL0S_RX => State::Cl0sRx,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CL1 => State::Cl1,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CL2 => State::Cl2,
                        usb4::LANE_ADP_CS_1_ADAPTER_STATE_CLD => State::Cld,
                        _ => State::Unknown,
                    }
                } else {
                    State::Unknown
                }
            }

            Type::DisplayPortIn | Type::DisplayPortOut => {
                if let Some(reg) = self.register_by_name("ADP_DP_CS_0") {
                    if reg.flag("VE") && reg.flag("AE") {
                        State::Enabled
                    } else {
                        State::Disabled
                    }
                } else {
                    State::Unknown
                }
            }

            Type::PcieDown | Type::PcieUp => {
                if let Some(reg) = self.register_by_name("ADP_PCIE_CS_0") {
                    if reg.flag("PE") {
                        State::Enabled
                    } else {
                        State::Disabled
                    }
                } else {
                    State::Unknown
                }
            }

            Type::Usb3Down | Type::Usb3Up => {
                if let Some(reg) = self.register_by_name("ADP_USB3_GX_CS_0") {
                    if reg.flag("PE") && reg.flag("V") {
                        State::Enabled
                    } else {
                        State::Disabled
                    }
                } else {
                    State::Unknown
                }
            }

            _ => State::Unknown,
        }
    }

    /// Returns slice of `self.regs` if capability is found
    fn find_cap(&self, cap_id: u16) -> Option<&[Register]> {
        if let Some(ref regs) = self.regs {
            let mut start: usize = 0;
            let mut end: usize = 0;

            for (i, reg) in regs.iter().enumerate() {
                if reg.cap_id == cap_id {
                    // Capabilities cannot start from 0
                    if start == 0 {
                        start = i;
                    }
                    if end < i {
                        end = i;
                    }
                }
            }

            if start == 0 || end <= start {
                return None;
            }

            Some(&regs[start..end])
        } else {
            None
        }
    }

    /// Returns number of this adapter.
    pub fn adapter(&self) -> u16 {
        self.adapter
    }

    /// Returns type of this adapter.
    pub fn kind(&self) -> Type {
        self.kind
    }

    /// Returns `true` if this is valid adapter (e.g implemented).
    pub fn is_valid(&self) -> bool {
        self.kind != Type::Inactive && self.kind != Type::Unknown
    }

    /// Returns current state of the adapter.
    pub fn state(&self) -> State {
        self.state
    }

    /// Is this lane (0 or 1) adapter.
    pub fn is_lane(&self) -> bool {
        self.kind == Type::Lane
    }

    /// Is this lane 0 adapter.
    pub fn is_lane0(&self) -> bool {
        if self.is_lane() {
            if self.usb4 && self.find_cap(usb4::ADP_CAP_ID_USB4).is_some() {
                return true;
            } else {
                // Thunderbolt 1-3 router so hard-coded adapter ordering.
                return self.adapter() == 1 || self.adapter() == 3;
            }
        }
        false
    }

    /// Is this lane 1 adapter.
    pub fn is_lane1(&self) -> bool {
        self.is_lane() && !self.is_lane0()
    }

    /// If the protocol adapter is enabled
    pub fn is_enabled(&self) -> bool {
        self.is_protocol() && self.state == State::Enabled
    }

    /// Returns `true` if this is protocol adapter.
    pub fn is_protocol(&self) -> bool {
        matches!(
            self.kind,
            Type::HostInterface
                | Type::DisplayPortIn
                | Type::DisplayPortOut
                | Type::PcieDown
                | Type::PcieUp
                | Type::Usb3Down
                | Type::Usb3Up
        )
    }

    /// Returns `true` if this is upstream adapter of the router.
    pub fn is_upstream(&self) -> bool {
        self.upstream
    }

    /// Returns minimum input `HopID` of this adapter.
    pub fn min_hop(&self) -> Option<u8> {
        match self.kind {
            Type::HostInterface => Some(1),

            Type::Lane
            | Type::DisplayPortIn
            | Type::DisplayPortOut
            | Type::PcieDown
            | Type::PcieUp
            | Type::Usb3Down
            | Type::Usb3Up => Some(8),

            _ => None,
        }
    }

    /// Reads the adapter register space.
    ///
    /// Must be called before accessing any other register space.
    pub fn read_registers(&mut self) -> Result<()> {
        let mut path_buf = PathBuf::from(&self.debugfs_path);
        path_buf.push(format!("port{}", self.adapter()));
        path_buf.push(DEBUGFS_REGS);

        let mut regs = match read(&path_buf, None, None) {
            Ok(regs) => regs,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        };

        // Figure out the type of the adapter from the registers now.
        self.kind = Self::parse_kind(&regs);

        // Pull in metadata.
        for reg in &mut regs {
            let metadata =
                Metadata::from_adapter_type_and_offset("adapter", self.kind(), reg.relative_offset);
            if let Some(metadata) = metadata {
                reg.set_metadata(metadata);
            }
        }

        // Assign new registers.
        self.regs = Some(regs);

        // At this point we have full metadata available for the registers so we can update the
        // initial state.
        self.state = self.parse_state();

        Ok(())
    }

    /// Reads registers if they are not read already.
    ///
    /// If they are already read this does nothing. You can force re-read by calling
    /// [`read_registers()`](Self::read_registers).
    pub fn read_registers_cached(&mut self) -> Result<()> {
        if self.regs.is_none() {
            return self.read_registers();
        }
        Ok(())
    }

    /// Returns adapter config space registers.
    pub fn registers(&self) -> Option<&Vec<Register>> {
        self.regs.as_ref()
    }

    /// Returns adapter config space register by name.
    ///
    /// The match is case insensitive.
    pub fn register_by_name(&self, name: &str) -> Option<&Register> {
        self.regs.as_ref()?.iter().find(|r| {
            r.name()
                .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        })
    }

    /// Returns mutable reference to an adapter config space register by name.
    ///
    /// The match is case insensitive.
    pub fn register_by_name_mut(&mut self, name: &str) -> Option<&mut Register> {
        self.regs.as_mut()?.iter_mut().find(|r| {
            r.name()
                .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        })
    }

    /// Returns adapter config space register by absolute offset.
    pub fn register_by_offset(&self, offset: u16) -> Option<&Register> {
        self.regs
            .as_ref()?
            .iter()
            .find(|r| r.offset == offset as u32)
    }

    /// Returns mutable reference to an adapter config space register by absolute offset.
    pub fn register_by_offset_mut(&mut self, offset: u16) -> Option<&mut Register> {
        self.regs
            .as_mut()?
            .iter_mut()
            .find(|r| r.offset == offset as u32)
    }

    /// Reads adapter path config space.
    ///
    /// This must be called before accessing path config space registers.
    pub fn read_paths(&mut self) -> Result<()> {
        let mut path_buf = PathBuf::from(&self.debugfs_path);
        path_buf.push(format!("port{}", self.adapter()));
        path_buf.push(DEBUGFS_PATH);

        self.path_regs = match read(&path_buf, None, None) {
            Ok(path_regs) => Some(path_regs),
            // Lane 1 adapter path config space is not accessible in USB4 v2 devices so this is
            // fine, just return empty paths.
            Err(_) if self.is_lane1() => return Ok(()),
            Err(err) => return Err(err),
        };

        // Pull in metadata.
        let kind = self.kind();

        if let Some(path_regs) = &mut self.path_regs {
            for reg in path_regs.iter_mut() {
                let metadata =
                    Metadata::from_adapter_type_and_offset("path", kind, reg.relative_offset);
                if let Some(metadata) = metadata {
                    reg.set_metadata(metadata);
                }
            }
        }

        if let Some(in_min_hop) = self.min_hop() {
            let mut paths = Vec::new();

            for p in self
                .path_regs
                .as_ref()
                .unwrap()
                .iter()
                .filter(|p| p.offset >= (in_min_hop * 2).into() && p.offset % 2 == 0)
            {
                let in_hop: u16 = (p.offset / 2) as u16;

                if let Some(path) = Path::from(self.adapter, in_hop, p.value) {
                    paths.push(path);
                }
            }

            if !paths.is_empty() {
                self.paths = Some(paths);
            } else {
                self.paths = None;
            }
        }

        Ok(())
    }

    /// Reads path config spaces if not read already.
    ///
    /// If they are already read this does nothing. You can force re-read by calling
    /// [`read_paths()`](Self::read_paths).
    pub fn read_paths_cached(&mut self) -> Result<()> {
        if self.path_regs.is_none() {
            return self.read_paths();
        }
        Ok(())
    }

    /// Returns path config space registers.
    pub fn path_registers(&self) -> Option<&Vec<Register>> {
        self.path_regs.as_ref()
    }

    /// Returns path config space register by absolute offset.
    pub fn path_register_by_offset(&self, offset: u16) -> Option<&Register> {
        self.path_regs
            .as_ref()?
            .iter()
            .find(|r| r.offset == offset as u32)
    }

    /// Returns mutable reference to a path config space register by absolute offset.
    pub fn path_register_by_offset_mut(&mut self, offset: u16) -> Option<&mut Register> {
        self.path_regs
            .as_mut()?
            .iter_mut()
            .find(|r| r.offset == offset as u32)
    }

    /// Returns enabled path in `in_hop`.
    pub fn path(&self, in_hop: u16) -> Option<&Path> {
        self.paths
            .as_ref()
            .and_then(|paths| paths.iter().find(|p| p.in_hop == in_hop))
    }

    /// Returns all enabled paths of this adapter.
    pub fn paths(&self) -> Option<&Vec<Path>> {
        self.paths.as_ref()
    }

    /// Reads adapter counters config space.
    ///
    /// Must be called before accessing adapter counter registers.
    pub fn read_counters(&mut self) -> Result<()> {
        let mut path_buf = PathBuf::from(&self.debugfs_path);
        path_buf.push(format!("port{}", self.adapter()));
        path_buf.push(DEBUGFS_COUNTERS);

        self.counter_regs = match read(&path_buf, None, None) {
            Ok(counter_regs) => Some(counter_regs),
            // Lane 1 adapter counters config space is not accessible in USB4 v2 devices so this is
            // fine, just return empty paths.
            Err(_) if self.is_lane1() => return Ok(()),
            Err(err) => return Err(err),
        };

        Ok(())
    }

    /// Reads counters if they are not read already.
    ///
    /// If they are already read this does nothing. You can force re-read by calling
    /// [`read_counters()`](Self::read_counters).
    pub fn read_counters_cached(&mut self) -> Result<()> {
        if self.counter_regs.is_none() {
            return self.read_counters();
        }
        Ok(())
    }

    /// Returns adapter counter registers.
    pub fn counter_registers(&self) -> Option<&Vec<Register>> {
        self.counter_regs.as_ref()
    }

    /// Returns adapter counter register by absolute offset.
    pub fn counter_register_by_offset(&self, offset: u16) -> Option<&Register> {
        self.counter_regs
            .as_ref()?
            .iter()
            .find(|r| r.offset == offset as u32)
    }

    /// Returns mutable reference to a counter register by absolute offset.
    pub fn counter_register_by_offset_mut(&mut self, offset: u16) -> Option<&mut Register> {
        self.counter_regs
            .as_mut()?
            .iter_mut()
            .find(|r| r.offset == offset as u32)
    }

    /// Clears all counters. This takes effect immediately.
    pub fn clear_counters(&mut self) -> Result<()> {
        let mut path_buf = PathBuf::from(&self.debugfs_path);
        path_buf.push(format!("port{}", self.adapter()));
        path_buf.push(DEBUGFS_COUNTERS);

        let file = OpenOptions::new().write(true).open(path_buf)?;
        let mut writer = BufWriter::new(file);

        // Empty line clear all counters.
        writeln!(&mut writer)?;

        writer.flush()
    }

    /// Writes all changed registers in all configuration spaces back to the hardware. After this
    /// you should call [`read_registers()`](Self::read_registers()),
    /// [`read_paths()`](Self::read_paths()) and [`read_counters()`](Self::read_counters()) to
    /// re-read registers from the hardware.
    pub fn write_changed(&mut self) -> Result<()> {
        let mut path_buf = PathBuf::from(&self.debugfs_path);
        path_buf.push(format!("port{}", self.adapter()));

        if let Some(regs) = &self.regs {
            path_buf.push(DEBUGFS_REGS);
            write_changed(&path_buf, regs)?;
            path_buf.pop();
        }

        if let Some(path_regs) = &self.path_regs {
            path_buf.push(DEBUGFS_PATH);
            write_changed(&path_buf, path_regs)?;
            path_buf.pop();
        }

        if let Some(counter_regs) = &self.counter_regs {
            path_buf.push(DEBUGFS_COUNTERS);
            write_changed(&path_buf, counter_regs)?;
            path_buf.pop();
        }

        Ok(())
    }
}

impl Device {
    /// Return debugfs path.
    pub fn debugfs_path(&self) -> PathBuf {
        let mut path_buf = PathBuf::from(DEBUGFS_ROOT);
        path_buf.push(self.kernel_name());
        path_buf
    }

    /// Return `true` if device registers are writable.
    pub fn registers_writable(&self) -> bool {
        if let Ok(path_buf) = router_path_buf(self) {
            if let Ok(file) = File::open(path_buf) {
                if let Ok(metadata) = file.metadata() {
                    return !metadata.permissions().readonly();
                }
            }
        }
        false
    }

    /// Read registers from hardware.
    pub fn read_registers(&mut self) -> Result<()> {
        let mut regs = read(&router_path_buf(self)?, None, None)?;

        // Pull in metadata.
        for reg in &mut regs {
            if let Some(metadata) = Metadata::from_router_offset(reg.relative_offset) {
                reg.set_metadata(metadata);
            }
        }

        self.regs = Some(regs);

        Ok(())
    }

    /// Reads the router register space if not already read.
    ///
    /// If [`read_registers()`](Self::read_registers()) is already called does nothing. Otherwise
    /// calls it first.
    pub fn read_registers_cached(&mut self) -> Result<()> {
        if self.regs.is_none() {
            return self.read_registers();
        }
        Ok(())
    }

    /// Returns all device registers.
    pub fn registers(&self) -> Option<&Vec<Register>> {
        self.regs.as_ref()
    }

    /// Get a single register by name.
    ///
    /// The match is case insensitive.
    pub fn register_by_name(&self, name: &str) -> Option<&Register> {
        self.regs.as_ref()?.iter().find(|r| {
            r.name()
                .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        })
    }

    /// Get a single mutable register by name.
    ///
    /// The match is case insensitive.
    pub fn register_by_name_mut(&mut self, name: &str) -> Option<&mut Register> {
        self.regs.as_mut()?.iter_mut().find(|r| {
            r.name()
                .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        })
    }

    /// Get a single register by offset.
    pub fn register_by_offset(&self, offset: u16) -> Option<&Register> {
        self.regs
            .as_ref()?
            .iter()
            .find(|r| r.offset == offset as u32)
    }

    /// Get a single mutable register by offset.
    pub fn register_by_offset_mut(&mut self, offset: u16) -> Option<&mut Register> {
        self.regs
            .as_mut()?
            .iter_mut()
            .find(|r| r.offset == offset as u32)
    }

    fn read_adapter(&self, adapter: u16, upstream: bool) -> Result<Adapter> {
        let mut path_buf = path_buf()?;
        path_buf.push(self.kernel_name());

        Ok(Adapter::new(
            adapter,
            path_buf,
            self.usb4_version().is_some(),
            upstream,
        ))
    }

    /// Return max adapter number.
    ///
    /// Only for routers. [`read_registers()`](Self::read_registers()) must be called before this.
    pub fn max_adapter(&self) -> Option<u16> {
        if !self.is_router() {
            return None;
        }

        let reg = self.register_by_name("ROUTER_CS_1")?;
        Some(reg.field("Max Adapter") as u16)
    }

    /// Returns upstream adapter number of this device.
    ///
    /// Only for routers. [`read_registers()`](Self::read_registers()) must be called before this.
    pub fn upstream_adapter(&self) -> Option<u16> {
        if !self.is_router() {
            return None;
        }

        let reg = self.register_by_name("ROUTER_CS_1")?;
        Some(reg.field("Upstream Adapter") as u16)
    }

    /// Reads device adapters from `debugfs`.
    ///
    /// Calls [`read_registers()`](Self::read_registers()) if it has not been called for the
    /// device. For each adapter also calls [`read_registers()`](Adapter::read_registers()). Must
    /// be called before accessing device adapters.
    pub fn read_adapters(&mut self) -> Result<()> {
        if !self.is_router() {
            return Err(Error::new(ErrorKind::InvalidData, "router expected"));
        }

        // Need to call this in order to figure out max adapter and upstream adapter.
        self.read_registers_cached()?;

        let max_adapter = self.max_adapter().unwrap();
        let upstream_adapter = self.upstream_adapter().unwrap();
        let mut adapters = Vec::new();

        for i in 1..=max_adapter {
            let mut adapter = self.read_adapter(i, i == upstream_adapter)?;
            adapter.read_registers()?;
            adapters.push(adapter);
        }

        self.adapters = Some(adapters);

        Ok(())
    }

    /// Reads device adapters if they are not read already.
    ///
    /// If they are read does nothing. You can force re-read by calling
    /// [`read_adapters()`](Self::read_adapters).
    pub fn read_adapters_cached(&mut self) -> Result<()> {
        if self.adapters.is_none() {
            return self.read_adapters();
        }
        Ok(())
    }

    /// Returns device adapter with given number.
    pub fn adapter(&self, adapter_num: u16) -> Option<&Adapter> {
        self.adapters
            .as_ref()?
            .iter()
            .find(|a| a.adapter == adapter_num)
    }

    /// Returns mutable reference to an adapter with given number.
    pub fn adapter_mut(&mut self, adapter_num: u16) -> Option<&mut Adapter> {
        self.adapters
            .as_mut()?
            .iter_mut()
            .find(|a| a.adapter == adapter_num)
    }

    /// Returns all device adapters.
    ///
    /// Only valid to call this if [`read_adapters()`](Self::read_adapters()) has been called
    /// first.
    pub fn adapters(&self) -> Option<&Vec<Adapter>> {
        self.adapters.as_ref()
    }

    /// Returns mutable reference to all device adapters.
    ///
    /// Only valid to call this if [`read_adapters()`](Self::read_adapters()) has been called
    /// first.
    pub fn adapters_mut(&mut self) -> Option<&mut Vec<Adapter>> {
        self.adapters.as_mut()
    }

    /// Writes all changed registers back to the hardware.
    ///
    /// After this is called it is recommended to call
    /// [`read_registers()`](Self::read_registers()) to refresh the registers from the hardware.
    pub fn write_changed(&mut self) -> Result<()> {
        if let Some(regs) = &self.regs {
            write_changed(&router_path_buf(self)?, regs)?;
        }

        Ok(())
    }
}
