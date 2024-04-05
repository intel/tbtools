// Thunderbolt/USB4 debug tools.
//
// Copyright (C) 2024, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Trace the transport layer configuration packet traffic.
//!
//! This module allows enabling and disabling [tracepoints] in the kernel driver. The kernel must be
//! compiled with `CONFIG_TRACING=y` set. The driver exposes all the transport layer configuration
//! packet traffic through [tracepoints]. This module allows easy programmatic access to them.
//!
//! # Examples
//! The following program enables tracing programmatically.
//!
//! ```no_run
//! # use std::{io, process};
//! use tbtools::trace;
//!
//! # fn main() -> io::Result<()> {
//! if !trace::supported() {
//!     eprintln!("tracing is not supported");
//!     process::exit(1);
//! }
//!
//! trace::enable()?;
//! # Ok(())
//! # }
//! ```
//! The next example dumps the contents of the live trace buffer to `stdout`.
//!
//! ```no_run
//! # use std::io;
//! use tbtools::trace;
//!
//! # fn main() -> io::Result<()> {
//! for entry in trace::live_buffer()? {
//!     println!("task: {} pid: {}", entry.task(), entry.pid());
//!
//!     let packet = entry.packet();
//!     assert!(packet.is_some());
//!     // Process the control packet.
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [tracepoints]: https://docs.kernel.org/trace/events.html

use crate::{
    debugfs::{BitField, BitFields, Name},
    genmask, util, Address, ConfigSpace, Pdf,
};
use lazy_static::lazy_static;
use nix::sys::time::TimeVal;
use regex::Regex;
use serde_json::Value;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Result, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const TRACEFS_ROOT: &str = "/sys/kernel/debug/tracing";
const TRACEFS_TRACE: &str = "trace";
const TRACEFS_CURRENT_TRACER: &str = "current_tracer";
const TRACEFS_TRACE_CLOCK: &str = "trace_clock";
const TRACEFS_EVENTS: &str = "events";
const TRACEFS_EVENTS_THUNDERBOLT: &str = "thunderbolt";
const TRACEFS_EVENTS_ENABLE: &str = "enable";
const TRACEFS_EVENTS_FILTER: &str = "filter";

/// Notification Events.
///
/// These match directly the codes in USB4 v2.0 table 6-12.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Event Code is not known.
    Unknown(u8),
    /// ERR_CONN.
    ErrConn,
    /// ERR_LINK.
    ErrLink,
    /// ERR_ADDR.
    ErrAddr,
    /// ERR_ADP.
    ErrAdp,
    /// ERR_ENUM.
    ErrEnum,
    /// ERR_NUA.
    ErrNua,
    /// ERR_LEN.
    ErrLen,
    /// ERR_HEC.
    ErrHec,
    /// ERR_FC.
    ErrFc,
    /// ERR_PLUG (Plug/Unplug Event).
    ErrPlug,
    /// ERR_LOCK.
    ErrLock,
    /// HP_ACK.
    HpAck,
    /// ROP_CMPLT.
    RopCmplt,
    /// POP_CMPLT.
    PopCmplt,
    /// PCIE_WAKE.
    PcieWake,
    /// DP_CON_CHANGE.
    DpConChange,
    /// LINK_RECOVERY.
    LinkRecovery,
    /// ASYM_LINK.
    AsymLink,
    /// DP_BW.
    DpBw,
    /// DPTX_DISCOVERY.
    DptxDiscovery,
}

impl Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match *self {
            Self::ErrConn => "ERR_CONN",
            Self::ErrLink => "ERR_LINK",
            Self::ErrAddr => "ERR_ADDR",
            Self::ErrAdp => "ERR_ADP",
            Self::ErrEnum => "ERR_ENUM",
            Self::ErrNua => "ERR_NUA",
            Self::ErrLen => "ERR_LEN",
            Self::ErrHec => "ERR_HEC",
            Self::ErrFc => "ERR_FC",
            Self::ErrPlug => "ERR_PLUG",
            Self::ErrLock => "ERR_LOCK",
            Self::HpAck => "HP_ACK",
            Self::RopCmplt => "ROP_CMPLT",
            Self::PopCmplt => "POP_CMPLT",
            Self::PcieWake => "PCIE_WAKE",
            Self::DpConChange => "DP_CON_CHANGE",
            Self::LinkRecovery => "DP_CON_CHANGE",
            Self::AsymLink => "ASYM_LINK",
            Self::DpBw => "DP_BW",
            Self::DptxDiscovery => "DPTX_DISCOVERY",
            Self::Unknown(code) => {
                return write!(f, "{}", code);
            }
        };
        write!(f, "{}", s)
    }
}

impl From<u8> for Event {
    fn from(ec: u8) -> Self {
        match ec {
            0 => Self::ErrConn,
            1 => Self::ErrLink,
            2 => Self::ErrAddr,
            4 => Self::ErrAdp,
            7 => Self::HpAck,
            8 => Self::ErrEnum,
            9 => Self::ErrNua,
            11 => Self::ErrLen,
            12 => Self::ErrHec,
            13 => Self::ErrFc,
            14 => Self::ErrPlug,
            15 => Self::ErrLock,
            32 => Self::DpBw,
            33 => Self::RopCmplt,
            34 => Self::PopCmplt,
            35 => Self::PcieWake,
            36 => Self::DpConChange,
            37 => Self::DptxDiscovery,
            38 => Self::LinkRecovery,
            39 => Self::AsymLink,
            _ => Self::Unknown(ec),
        }
    }
}

#[derive(Debug)]
struct Metadata {
    offset: u16,
    name: Option<String>,
    bitfields: Option<Vec<BitField>>,
    packet_type: Option<String>,
}

impl Metadata {
    /// Double word offset inside the packet.
    pub fn offset(&self) -> u16 {
        self.offset
    }

    /// Returns "Packet Type" if this is such field.
    pub fn packet_type(&self) -> Option<&str> {
        self.packet_type.as_deref()
    }
}

fn parse_field_metadata(value: &Value) -> Option<Metadata> {
    let offset = value.get("offset")?.as_u64()? as u16;
    let name = value.get("name").and_then(|n| n.as_str()).map(String::from);
    let bitfields = BitField::parse(value);
    let packet_type = value
        .get("packet_type")
        .and_then(|n| n.as_str())
        .map(String::from);

    Some(Metadata {
        offset,
        name,
        bitfields,
        packet_type,
    })
}

fn parse_fields_metadata(value: &Value) -> Option<Vec<Metadata>> {
    let mut metadata = Vec::new();

    for v in value.as_array()? {
        metadata.push(parse_field_metadata(v)?);
    }

    Some(metadata)
}

fn parse_xdomain_metadata(value: &Value) -> Option<HashMap<Uuid, Vec<Metadata>>> {
    let json = value.as_object()?;

    let mut metadata = HashMap::new();

    for key in json.keys() {
        let uuid = if key == "header" {
            Uuid::nil()
        } else {
            Uuid::parse_str(key).ok()?
        };

        let field_metadata = parse_fields_metadata(json.get(key)?)?;
        metadata.insert(uuid, field_metadata);
    }

    Some(metadata)
}

fn parse_control_metadata(value: &Value) -> Option<HashMap<u32, Vec<Metadata>>> {
    let mut metadata = HashMap::new();

    let values = value.as_array()?;
    for v in values {
        let pdf = v.get("pdf")?.as_u64()? as u32;
        let field_metadata = parse_fields_metadata(v.get("fields")?)?;

        metadata.insert(pdf, field_metadata);
    }

    Some(metadata)
}

lazy_static! {
    // Pull in the field descriptions.
    static ref CONTROL_FIELDS: Value = serde_json::from_str(
        include_str!("data/control.json")
    )
    .unwrap();

    static ref CONTROL_METADATA: HashMap<u32, Vec<Metadata>> =
        parse_control_metadata(&CONTROL_FIELDS).unwrap();

    static ref XDOMAIN_FIELDS: Value = serde_json::from_str(
        include_str!("data/xdomain.json")
    )
    .unwrap();

    static ref XDOMAIN_METADATA: HashMap<Uuid, Vec<Metadata>> =
        parse_xdomain_metadata(&XDOMAIN_FIELDS).unwrap();
}

/// Single double word field inside a [`ControlPacket`].
///
/// If packet type is known the field may be split into several [`BitField`]s that can be acccessed
/// separately.
#[derive(Debug)]
pub struct Field<'a> {
    offset: u16,
    value: u32,
    metadata: Option<&'a Metadata>,
}

impl Field<'_> {
    /// Returns double word offset inside the packet.
    pub fn offset(&self) -> u16 {
        self.offset
    }

    /// Returns value of the field.
    pub fn value(&self) -> u32 {
        self.value
    }
}

impl Name for Field<'_> {
    /// Returns name of the field if known.
    fn name(&self) -> Option<&str> {
        self.metadata?.name.as_deref()
    }
}

impl BitFields<u32> for Field<'_> {
    /// Returns fields if this [`Field`] contains [`BitField`]s.
    fn fields(&self) -> Option<&Vec<BitField>> {
        self.metadata?.bitfields.as_ref()
    }

    /// Returns value of the given field.
    fn field_value(&self, field: &BitField) -> u32 {
        let mask = genmask!(*field.range().end() as u32, *field.range().start() as u32);
        let shift = *field.range().start();

        (self.value & mask) >> shift
    }
}

/// Transport layer control packet parsed from the trace [`Entry`].
///
/// The contained information except of [`ControlPacket::pdf()`} is completely parsed from the double
/// word data that is coming from the [`Entry::data()`].
pub struct ControlPacket<'a> {
    pdf: Pdf,
    fields: Vec<Field<'a>>,
    data_start: Option<u16>,
    data: Option<&'a [u32]>,
    uuid: Option<Uuid>,
}

impl<'a> ControlPacket<'a> {
    /// Returns [`Pdf`] of this packet.
    pub fn pdf(&self) -> Pdf {
        self.pdf
    }

    /// Returns parsed route string.
    ///
    /// This is unmodified route string that includes also the `CM` bit if it is set.
    pub fn route(&self) -> u64 {
        let values: Vec<u64> = self.fields[0..=1].iter().map(|f| f.value as u64).collect();
        values[0] << 32 | values[1]
    }

    /// Is the CM bit set in the route string?
    pub fn cm(&self) -> bool {
        self.route() & 1 << 63 != 0
    }

    /// Returns `Adapter Num` from the packet.
    pub fn adapter_num(&self) -> Option<u16> {
        Some(
            self.field_by_bitfield_name("Adapter Num")?
                .field("Adapter Num") as u16,
        )
    }

    /// Returns `Address` field of Read/Write packet and `Offset` field of Inter-Domain packet.
    /// This is the start register address or the directory block offset.
    pub fn data_address(&self) -> Option<u16> {
        if self.is_xdomain() {
            Some(self.field_by_bitfield_name("Offset")?.field("Offset") as u16)
        } else {
            Some(self.field_by_bitfield_name("Address")?.field("Address") as u16)
        }
    }

    /// Returns the double word offset inside this packet where the first data double word is.
    pub fn data_start(&self) -> Option<u16> {
        self.data_start
    }

    /// Returns `Read Size` or `Write Size` of Read/Write packet and `Properties Block Size`
    /// Inter-Domain packet.
    pub fn data_size(&self) -> Option<u16> {
        if self.is_read() {
            Some(self.field_by_bitfield_name("Read Size")?.field("Read Size") as u16)
        } else if self.is_write() {
            Some(
                self.field_by_bitfield_name("Write Size")?
                    .field("Write Size") as u16,
            )
        } else if self.is_xdomain() {
            Some(
                self.field_by_bitfield_name("Properties Block Size")?
                    .field("Properties Block SizeData Size") as u16,
            )
        } else {
            None
        }
    }

    /// Returns the raw data inside the packet.
    ///
    /// For Intra-domain packets this is the actual Read/Write configuration space data, and for
    /// Inter-domain packets this is the properties block data.
    pub fn data(&self) -> Option<&[u32]> {
        self.data
    }

    /// Returns [`true`] if this is Read packet.
    pub fn is_read(&self) -> bool {
        self.pdf() == Pdf::ReadRequest || self.pdf() == Pdf::ReadResponse
    }

    /// Returns [`true`] if this is Write packet.
    pub fn is_write(&self) -> bool {
        self.pdf() == Pdf::WriteRequest || self.pdf() == Pdf::WriteResponse
    }

    /// Returns [`true`] if this is Inter-Domain packet.
    pub fn is_xdomain(&self) -> bool {
        self.pdf() == Pdf::XdomainRequest || self.pdf() == Pdf::XdomainResponse
    }

    /// Returns Inter-Domain protocol UUID.
    pub fn uuid(&self) -> Option<&Uuid> {
        self.uuid.as_ref()
    }

    /// Returns UUID from Inter-Domain packet by given name.
    pub fn uuid_by_name(&self, name: &str) -> Option<Uuid> {
        let uuid: Vec<_> = self
            .fields
            .iter()
            .filter(|f| {
                if let Some(n) = f.name() {
                    n == name
                } else {
                    false
                }
            })
            .map(|f| f.value)
            .collect();

        util::u32_to_uuid(&uuid)
    }

    /// Returns the `Packet Type` field from Inter-Domain packet.
    pub fn packet_type(&self) -> Option<(u32, &str)> {
        if let Some(field) = self.fields.iter().find(|f| f.has_field("Packet Type")) {
            let bitfield = field.field_by_name("Packet Type").unwrap();
            let value = field.field_value(bitfield);

            Some((value, bitfield.value_name(value).unwrap()))
        } else {
            None
        }
    }

    /// Returns all fields in this packet.
    pub fn fields(&self) -> &Vec<Field> {
        &self.fields
    }

    /// Returns packet field by name.
    pub fn field_by_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name() == Some(name))
    }

    /// Returns packet field by double word offset.
    pub fn field_by_offset(&self, offset: u16) -> Option<&Field> {
        self.fields.get(offset as usize)
    }

    /// Returns field that has bitfield with given name.
    pub fn field_by_bitfield_name(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.has_field(name))
    }

    fn find_xdomain_metadata(
        metadata: &'a [Metadata],
        offset: u16,
        kind: Option<&str>,
    ) -> Option<&'a Metadata> {
        let md = metadata
            .iter()
            .filter(|m| m.offset() == offset)
            .find(|m| m.packet_type() == kind);
        if md.is_some() {
            return md;
        }
        metadata
            .iter()
            .find(|m| m.packet_type().is_none() && m.offset() == offset)
    }

    fn parse_xdomain_header(data: &'a [u32], fields: &mut [Field<'a>]) {
        let metadata = &XDOMAIN_METADATA.get(&Uuid::nil()).unwrap();

        for (i, _) in data.iter().enumerate() {
            if let Some(m) = Self::find_xdomain_metadata(metadata, i as u16, None) {
                if let Some(field) = fields.get_mut(i) {
                    // Only set if None.
                    if field.metadata.is_none() {
                        field.metadata = Some(m);
                    }
                }
            }
        }
    }

    fn parse_xdomain_data(uuid: &Uuid, dwords: &'a [u32], fields: &mut [Field<'a>]) {
        let metadata = &XDOMAIN_METADATA.get(uuid);
        if metadata.is_none() {
            return;
        }
        let metadata = metadata.unwrap();
        let mut kind: Option<String> = None;

        for (i, _) in dwords.iter().enumerate() {
            if let Some(m) = Self::find_xdomain_metadata(metadata, i as u16, kind.as_deref()) {
                if let Some(field) = fields.get_mut(i) {
                    // Only set if None.
                    if field.metadata.is_none() {
                        field.metadata = Some(m);
                    }

                    // Is this the "Packet Type" bitfield.
                    if kind.is_none() {
                        if let Some(bitfield) = field.field_by_name("Packet Type") {
                            kind = bitfield
                                .value_name(field.field_value(bitfield))
                                .map(|v| v.to_string());
                        }
                    }
                }
            }
        }
    }

    fn parse_xdomain(dwords: &'a [u32], fields: &mut [Field<'a>]) -> Option<Uuid> {
        Self::parse_xdomain_header(dwords, fields);

        let uuid: Vec<_> = fields
            .iter()
            .filter(|f| {
                if let Some(name) = f.name() {
                    name == "UUID"
                } else {
                    false
                }
            })
            .map(|f| f.value)
            .collect();

        let uuid = util::u32_to_uuid(&uuid)?;
        Self::parse_xdomain_data(&uuid, dwords, fields);

        Some(uuid)
    }

    fn parse(pdf: Pdf, dwords: &'a [u32]) -> Option<Self> {
        let metadata = &CONTROL_METADATA.get(&pdf.to_num()?).unwrap();
        let mut fields = Vec::new();

        for (i, d) in dwords.iter().enumerate() {
            let m = metadata.iter().find(|m| m.offset() == i as u16);

            fields.push(Field {
                offset: i as u16,
                value: *d,
                metadata: m,
            });
        }

        let mut data_start = None;
        let mut data = None;
        let mut uuid = None;

        match pdf {
            Pdf::ReadResponse | Pdf::WriteRequest => {
                let offset = metadata.len();
                data_start = Some(offset as u16);
                data = Some(&dwords[offset..]);
            }

            Pdf::XdomainRequest | Pdf::XdomainResponse => {
                uuid = Self::parse_xdomain(dwords, &mut fields);
                if let Some(field) = fields
                    .iter()
                    .find(|f| f.has_field("Properties Block Generation"))
                {
                    let offset = field.offset() + 1;
                    data_start = Some(offset);
                    data = Some(&dwords[offset as usize..]);
                }
            }

            _ => (),
        }

        Some(Self {
            pdf,
            fields,
            data_start,
            data,
            uuid,
        })
    }
}

/// Single parsed trace line.
///
/// Each parsed trace line returned by [`Buffer::iter()`] contains one [`Entry`] that holds the
/// trace information.
pub struct Entry {
    task: String,
    pid: u32,
    cpu: u16,
    timestamp: TimeVal,
    function: String,
    pdf: Pdf,
    size: u16,
    dropped: bool,
    domain_index: u32,
    route: u64,
    offset: Option<u16>,
    event: Option<Event>,
    dwords: Option<u16>,
    adapter_num: Option<u16>,
    cs: Option<ConfigSpace>,
    sn: Option<u8>,
    unplug: Option<u8>,
    data: Vec<u32>,
}

impl Entry {
    /// Name of the task that handled the packet.
    pub fn task(&self) -> &str {
        &self.task
    }

    /// PID of the process.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Which CPU number this packet was handled.
    pub fn cpu(&self) -> u16 {
        self.cpu
    }

    /// Timestamp of the trace entry.
    pub fn timestamp(&self) -> &TimeVal {
        &self.timestamp
    }

    /// Returns the kernel function this packet originated from.
    pub fn function(&self) -> &str {
        &self.function
    }

    /// Returns Protocol Defined Field of the packet.
    pub fn pdf(&self) -> Pdf {
        self.pdf
    }

    /// Size of the packet in double words, not including CRC.
    pub fn size(&self) -> u16 {
        self.size
    }

    /// Was the packet dropped by the kernel?
    pub fn dropped(&self) -> bool {
        self.dropped
    }

    /// Returns domain number from the packet.
    pub fn domain_index(&self) -> u32 {
        self.domain_index
    }

    /// Returns route string from the packet.
    pub fn route(&self) -> u64 {
        self.route
    }

    /// Returns [`Event`] if this is notification packet.
    pub fn event(&self) -> Option<Event> {
        self.event
    }

    /// Returns double word offset in the packet.
    pub fn offset(&self) -> Option<u16> {
        self.offset
    }

    /// Returns number of double words to read or write.
    pub fn dwords(&self) -> Option<u16> {
        self.dwords
    }

    /// Adapter number if the packet was targeted to an adapter.
    pub fn adapter_num(&self) -> Option<u16> {
        match self.event() {
            // These two carry other than adapter number in "Event Info" field.
            Some(Event::RopCmplt) | Some(Event::DpConChange) => None,
            _ => self.adapter_num,
        }
    }

    /// Configuration space.
    pub fn cs(&self) -> Option<ConfigSpace> {
        self.cs
    }

    /// Sequence number from the trace entry.
    pub fn sn(&self) -> Option<u8> {
        self.sn
    }

    /// Is this unplug packet?
    pub fn unplug(&self) -> Option<bool> {
        Some(self.unplug? == 1)
    }

    /// Returns the data portion of the packet.
    pub fn data(&self) -> &[u32] {
        &self.data
    }

    /// Returns parsed control packet from the trace data.
    pub fn packet(&self) -> Option<ControlPacket> {
        ControlPacket::parse(self.pdf, self.data())
    }

    fn parse_task(s: &str) -> Option<String> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^([^-]+)-").unwrap();
        }
        let caps = RE.captures(s)?;
        Some(String::from(&caps[1]))
    }

    fn parse_pid(s: &str) -> Option<u32> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"^([^-]+)-(\d+)").unwrap();
        }
        let caps = RE.captures(s)?;
        caps[2].parse::<u32>().ok()
    }

    fn parse_cpu(s: &str) -> Option<u16> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"\[(\d\d\d)\]").unwrap();
        }
        let caps = RE.captures(s)?;
        caps[1].parse::<u16>().ok()
    }

    fn parse_timestamp(s: &str) -> Option<TimeVal> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"(\d+)\.(\d+):\s+").unwrap();
        }
        let caps = RE.captures(s)?;
        let seconds = caps[1].parse::<i64>().ok()?;
        let useconds = caps[2].parse::<i64>().ok()?;
        Some(TimeVal::new(seconds, useconds))
    }

    fn parse_function(s: &str) -> Option<String> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r":\s+(\w+)$").unwrap();
        }
        let caps = RE.captures(s)?;
        Some(String::from(&caps[1]))
    }

    fn parse_kv(s: &str) -> Option<HashMap<String, String>> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"\w+=(\w+|\[[^\]]+\])").unwrap();
        }
        let fields: Vec<_> = RE.find_iter(s).collect();
        let mut kv = HashMap::new();

        for field in fields {
            let (key, value) = field.as_str().split_once('=')?;
            kv.insert(String::from(key), String::from(value));
        }

        Some(kv)
    }

    fn parse_data(s: &str) -> Option<Vec<u32>> {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"\[([^\]]+)\]").unwrap();
        }
        let caps = RE.captures(s)?;
        let data: Vec<u32> = caps[1]
            .split(", ")
            .filter_map(util::parse_hex::<u32>)
            .collect();

        Some(data)
    }

    fn parse(line: &str) -> Option<Self> {
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        let (header, payload) = line.rsplit_once(':')?;
        let header = header.trim();
        let payload = payload.trim();

        let task = Self::parse_task(header)?;
        let pid = Self::parse_pid(header)?;
        let cpu = Self::parse_cpu(header)?;
        let timestamp = Self::parse_timestamp(header)?;
        let function = Self::parse_function(header)?;

        let kv = Self::parse_kv(payload)?;

        let pdf = match kv.get("type")?.as_str() {
            "TB_CFG_PKG_READ" if function == "tb_tx" => Pdf::ReadRequest,
            "TB_CFG_PKG_READ" if function == "tb_rx" => Pdf::ReadResponse,
            "TB_CFG_PKG_WRITE" if function == "tb_tx" => Pdf::WriteRequest,
            "TB_CFG_PKG_WRITE" if function == "tb_rx" => Pdf::WriteResponse,
            "TB_CFG_PKG_ERROR" => Pdf::Notification,
            "TB_CFG_PKG_EVENT" => Pdf::HotPlugEvent,
            "TB_CFG_PKG_NOTIFY_ACK" => Pdf::NotificationAck,
            "TB_CFG_PKG_XDOMAIN_REQ" => Pdf::XdomainRequest,
            "TB_CFG_PKG_XDOMAIN_RESP" => Pdf::XdomainResponse,
            "TB_CFG_PKG_ICM_EVENT" => Pdf::IcmEvent,
            "TB_CFG_PKG_ICM_CMD" => Pdf::IcmRequest,
            "TB_CFG_PKG_ICM_RESP" => Pdf::IcmResponse,
            _ => Pdf::Unknown,
        };

        let size = kv.get("size")?.parse::<u16>().ok()?;
        let dropped = kv
            .get("dropped")
            .map_or(0, |d| d.parse::<u8>().unwrap_or(0))
            == 1;
        let domain_index = kv.get("domain")?.parse::<u32>().ok()?;
        let route = util::parse_route(kv.get("route")?).ok()?;
        let offset = kv.get("offset").and_then(|o| util::parse_hex::<u16>(o));
        let event: Option<Event> = kv
            .get("error")
            .and_then(|o| util::parse_hex::<u8>(o))
            .map(|e| e.into());
        let dwords = kv.get("len").and_then(|l| l.parse::<u16>().ok());
        let adapter_num = kv.get("port").and_then(|p| p.parse::<u16>().ok());
        let cs: Option<ConfigSpace> = kv
            .get("config")
            .and_then(|cs| util::parse_hex::<u8>(cs))
            .map(|cs| cs.into());
        let sn = kv.get("seq").and_then(|s| s.parse::<u8>().ok());
        let unplug = kv.get("unplug").and_then(|s| util::parse_hex::<u8>(s));
        let data = Self::parse_data(kv.get("data")?)?;

        let entry = Self {
            task,
            pid,
            cpu,
            timestamp,
            function,
            pdf,
            size,
            dropped,
            domain_index,
            route,
            offset,
            event,
            dwords,
            adapter_num,
            cs,
            sn,
            unplug,
            data,
        };

        Some(entry)
    }
}

fn path_buf() -> Result<PathBuf> {
    let path_buf = PathBuf::from(TRACEFS_ROOT);

    if !path_buf.exists() {
        return Err(Error::from(ErrorKind::NotFound));
    }

    Ok(path_buf)
}

fn trace_events_thunderbolt_path(attr: &str) -> Result<PathBuf> {
    let mut path_buf = path_buf()?;

    path_buf.push(TRACEFS_EVENTS);
    path_buf.push(TRACEFS_EVENTS_THUNDERBOLT);
    path_buf.push(attr);

    Ok(path_buf)
}

/// Returns [`true`] if tracing of Thunderbolt/USB4 driver is supported.
///
/// If this returns [`false`] either the driver is not recent enough or you don't have
/// `CONFIG_TRACING=y` in your kernel configuration.
pub fn supported() -> bool {
    if let Ok(path_buf) = trace_events_thunderbolt_path(TRACEFS_EVENTS_ENABLE) {
        let file = File::open(path_buf);
        if file.is_ok() {
            return true;
        }
    }

    false
}

/// Adds tracing filter.
///
/// Allows filtering traffic that enters the trace buffer. Currently only supports
/// [`Address::Domain`].
pub fn add_filter(address: &Address) -> Result<()> {
    if let Address::Domain { domain } = address {
        let path_buf = trace_events_thunderbolt_path(TRACEFS_EVENTS_FILTER)?;
        let file = OpenOptions::new().append(true).open(path_buf)?;
        let mut writer = BufWriter::new(file);

        writeln!(&mut writer, "index == {}", domain)?;

        return writer.flush();
    }

    Err(Error::from(ErrorKind::InvalidInput))
}

/// Returns [`true`] if tracing is enabled.
pub fn enabled() -> bool {
    if let Ok(path_buf) = trace_events_thunderbolt_path(TRACEFS_EVENTS_ENABLE) {
        if let Ok(enable) = fs::read_to_string(path_buf) {
            if enable.trim() == "1" {
                return true;
            }
        }
    }

    false
}

fn set_trace_attribute(attribute: &str, value: &str) -> Result<()> {
    let mut path_buf = path_buf()?;
    path_buf.push(attribute);

    let file = OpenOptions::new().write(true).open(path_buf)?;
    let mut writer = BufWriter::new(file);

    writeln!(&mut writer, "{}", value)?;

    writer.flush()
}

fn set_current_tracer(tracer: &str) -> Result<()> {
    set_trace_attribute(TRACEFS_CURRENT_TRACER, tracer)
}

fn set_trace_clock(clock: &str) -> Result<()> {
    set_trace_attribute(TRACEFS_TRACE_CLOCK, clock)
}

fn do_enable(enable: bool) -> Result<()> {
    let path_buf = trace_events_thunderbolt_path(TRACEFS_EVENTS_ENABLE)?;
    let file = OpenOptions::new().write(true).open(path_buf)?;
    let mut writer = BufWriter::new(file);

    writeln!(&mut writer, "{}", if enable { "1" } else { "0" })?;

    writer.flush()
}

/// Enables tracing.
pub fn enable() -> Result<()> {
    set_current_tracer("nop")?;
    // Use global clock to make sure timestamps between CPUs are synchronized. This makes it easier
    // to correlate the kernel message buffer with the trace buffer.
    set_trace_clock("global")?;
    do_enable(true)
}

/// Disables tracing.
pub fn disable() -> Result<()> {
    do_enable(false)
}

/// Buffer that is parsed from the trace input/buffer.
///
/// You can create new one and then walk over each entry by calling [`iter()`](Self::iter).
pub struct Buffer {
    reader: BufReader<File>,
}

impl Buffer {
    /// Returns new [`Buffer`] parsed from given input path.
    fn new(path: &Path) -> Result<Self> {
        let file = OpenOptions::new().read(true).open(path)?;
        let reader = BufReader::new(file);

        Ok(Self { reader })
    }

    /// Returns iterator over parsed entries.
    pub fn iter(&self) -> &Self {
        self
    }
}

impl Iterator for Buffer {
    type Item = Entry;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();

        loop {
            let len = self.reader.read_line(&mut line).ok()?;
            if len == 0 {
                return None;
            }
            // Skip comments and anything that is not parsable.
            if let Some(entry) = Entry::parse(&line) {
                return Some(entry);
            }
            line.clear();
        }
    }
}

/// Takes the current buffer from `tracefs` and returns parsed [`Buffer`].
pub fn live_buffer() -> Result<Buffer> {
    let mut path_buf = path_buf()?;
    path_buf.push(TRACEFS_TRACE);
    Buffer::new(&path_buf)
}

/// Converts the input buffer into parsed [`Buffer`].
pub fn buffer(input: &Path) -> Result<Buffer> {
    Buffer::new(input)
}

/// Clears the current trace buffer.
pub fn clear() -> Result<()> {
    let mut path_buf = path_buf()?;
    path_buf.push(TRACEFS_TRACE);

    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path_buf)?;
    let mut writer = BufWriter::new(file);

    write!(&mut writer, "")?;
    writer.flush()
}

#[cfg(test)]
mod test {
    use super::*;

    const TRACE: &str = "
# tracer: nop
#
# entries-in-buffer/entries-written: 48757/66044   #P:14
     kworker/0:1-10      [000] .....    59.259648: tb_tx: type=TB_CFG_PKG_WRITE, size=7, domain=1, route=0, offset=0x1, len=4, port=0, config=0x2, seq=0, data=[0x00000000, 0x00000000, 0x04008001, 0x1003471b, 0x00000000, 0x80000000, 0x2000100a]
     kworker/0:1-10      [000] .....    59.259896: tb_tx: type=TB_CFG_PKG_READ, size=3, domain=1, route=0, offset=0xae, len=1, port=1, config=0x1, seq=0, data=[0x00000000, 0x00000000, 0x020820ae]
     kworker/0:1-10      [000] .....    59.260176: tb_tx: type=TB_CFG_PKG_READ, size=3, domain=1, route=0, offset=0xae, len=1, port=3, config=0x1, seq=0, data=[0x00000000, 0x00000000, 0x021820ae]
     kworker/7:1-164     [007] .....    59.266223: tb_rx: type=TB_CFG_PKG_READ, dropped=0, size=4, domain=0, route=0, offset=0x1b, len=1, port=7, config=0x2, seq=0, data=[0x80000000, 0x00000000, 0x0438201b, 0x4320033e]
     kworker/7:1-164     [007] .....    59.265381: tb_event: type=TB_CFG_PKG_EVENT, size=3, domain=0, route=0, port=5, unplug=0x0, data=[0x80000000, 0x00000000, 0x00000005]
     kworker/7:1-164     [007] .....    59.265405: tb_tx: type=TB_CFG_PKG_ERROR, size=3, domain=0, route=0, error=0x7, port=5, plug=0x2, data=[0x00000000, 0x00000000, 0x80000507]
   kworker/u28:0-11      [004] .....    60.944818: tb_tx: type=TB_CFG_PKG_READ, size=3, domain=1, route=1, offset=0x0, len=1, port=0, config=0x2, seq=0, data=[0x00000000, 0x00000001, 0x04002000]
   kworker/u28:0-11      [004] .....    60.944992: tb_tx: type=TB_CFG_PKG_READ, size=3, domain=1, route=1, offset=0x0, len=5, port=0, config=0x2, seq=0, data=[0x00000000, 0x00000001, 0x0400a000]
    kworker/12:1-134     [012] .....  5425.790705: tb_tx: type=TB_CFG_PKG_XDOMAIN_RESP, size=14, domain=1, route=1, data=[0x00000000, 0x00000001, 0x0000000b, 0x0ed738b6, 0xbb40ff42, 0xe290c297, 0x07ffb2c0, 0x00000002, 0x80877f49, 0x256a86f1, 0xffffffff, 0xffffffff, 0x00000000, 0x00000001]
   kworker/u28:0-11      [002] .....  5425.995295: tb_tx: type=TB_CFG_PKG_XDOMAIN_REQ, size=8, domain=1, route=1, data=[0x00000000, 0x00000001, 0x10000005, 0x0ed738b6, 0xbb40ff42, 0xe290c297, 0x07ffb2c0, 0x0000000c]
    kworker/12:1-134     [012] .....  5433.028883: tb_rx: type=TB_CFG_PKG_XDOMAIN_RESP, dropped=1, size=23, domain=1, route=1, data=[0x80000000, 0x00000001, 0x08000014, 0x9e588f79, 0x478a1636, 0x6456c697, 0xddc820a9, 0x000003d7, 0x186f7000, 0x4a0d7aa3, 0x1d925063, 0x80877f49, 0x256a86f1, 0xffffffff, 0xffffffff, 0x00000000, 0x00000002, 0x00000001, 0x00000008, 0x00000000, 0x00000000, 0x00000000, 0x00000000]
";

    #[test]
    fn skip_empty_and_comments() {
        let mut lines = TRACE.lines();
        let mut entry = Entry::parse(lines.next().unwrap());
        assert!(entry.is_none());
        entry = Entry::parse(lines.next().unwrap());
        assert!(entry.is_none());
        entry = Entry::parse(lines.next().unwrap());
        assert!(entry.is_none());
        entry = Entry::parse(lines.next().unwrap());
        assert!(entry.is_none());
        entry = Entry::parse(lines.next().unwrap());
        assert!(entry.is_some());
    }

    fn lines() -> impl Iterator<Item = &'static str> {
        TRACE
            .lines()
            .filter(|l| !l.starts_with("#") && !l.is_empty())
    }

    #[test]
    fn parse_all_valid() {
        for line in lines() {
            let entry = Entry::parse(line);
            assert!(entry.is_some());
        }
    }

    #[test]
    fn parse_write_request() {
        let entry = Entry::parse(lines().next().unwrap());
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/0:1");
        assert_eq!(entry.pid(), 10);
        assert_eq!(entry.cpu(), 0);
        assert_eq!(*entry.timestamp(), TimeVal::new(59, 259648));
        assert_eq!(entry.function(), "tb_tx");
        assert_eq!(entry.pdf(), Pdf::WriteRequest);
        assert_eq!(entry.size(), 7);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 1);
        assert_eq!(entry.offset(), Some(1));
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), Some(4));
        assert_eq!(entry.adapter_num(), Some(0));
        assert_eq!(entry.cs(), Some(ConfigSpace::Router));
        assert_eq!(entry.sn(), Some(0));
        let data = entry.data();
        assert_eq!(data.len(), 7);
        assert_eq!(data[0], 0x00000000);
        assert_eq!(data[1], 0x00000000);
        assert_eq!(data[2], 0x04008001);
        assert_eq!(data[3], 0x1003471b);
        assert_eq!(data[4], 0x00000000);
        assert_eq!(data[5], 0x80000000);
        assert_eq!(data[6], 0x2000100a);
        let packet = entry.packet();
        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert_eq!(packet.route(), 0);
        assert!(!packet.cm());
        assert!(packet.field_by_name("Route String High").is_some());
        assert_eq!(
            packet.field_by_name("Route String High").unwrap().value(),
            0
        );
        assert!(packet.field_by_name("Route String Low").is_some());
        assert_eq!(packet.field_by_name("Route String Low").unwrap().value(), 0);
        let bitfields = packet.field_by_offset(2);
        assert!(bitfields.is_some());
        let bitfields = bitfields.unwrap();
        assert!(bitfields.has_field("Address"));
        assert_eq!(bitfields.field("Address"), 1);
        assert!(bitfields.has_field("Write Size"));
        assert_eq!(bitfields.field("Write Size"), 4);
        assert!(bitfields.has_field("Adapter Num"));
        assert_eq!(bitfields.field("Adapter Num"), 0);
        assert!(bitfields.has_field("Configuration Space"));
        let bitfield = bitfields.field_by_name("Configuration Space").unwrap();
        assert_eq!(bitfields.field_value(bitfield), 2);
        assert_eq!(bitfield.value_name(2), Some("Router Configuration Space"));
    }

    #[test]
    fn parse_read_request() {
        let line = lines().nth(2).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/0:1");
        assert_eq!(entry.pid(), 10);
        assert_eq!(entry.cpu(), 0);
        assert_eq!(*entry.timestamp(), TimeVal::new(59, 260176));
        assert_eq!(entry.function(), "tb_tx");
        assert_eq!(entry.pdf(), Pdf::ReadRequest);
        assert_eq!(entry.size(), 3);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 1);
        assert_eq!(entry.route(), 0);
        assert_eq!(entry.offset(), Some(0xae));
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), Some(1));
        assert_eq!(entry.adapter_num(), Some(3));
        assert_eq!(entry.cs(), Some(ConfigSpace::Adapter));
        assert_eq!(entry.sn(), Some(0));
        let data = entry.data();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0], 0x00000000);
        assert_eq!(data[1], 0x00000000);
        assert_eq!(data[2], 0x021820ae);
    }

    #[test]
    fn parse_read_response() {
        let line = lines().nth(3).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/7:1");
        assert_eq!(entry.pid(), 164);
        assert_eq!(entry.cpu(), 7);
        assert_eq!(*entry.timestamp(), TimeVal::new(59, 266223));
        assert_eq!(entry.function(), "tb_rx");
        assert_eq!(entry.pdf(), Pdf::ReadResponse);
        assert_eq!(entry.size(), 4);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 0);
        assert_eq!(entry.route(), 0);
        assert_eq!(entry.offset(), Some(0x1b));
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), Some(1));
        assert_eq!(entry.adapter_num(), Some(7));
        assert_eq!(entry.cs(), Some(ConfigSpace::Router));
        assert_eq!(entry.sn(), Some(0));
        let data = entry.data();
        assert_eq!(data.len(), 4);
        assert_eq!(data[0], 0x80000000);
        assert_eq!(data[1], 0x00000000);
        assert_eq!(data[2], 0x0438201b);
        assert_eq!(data[3], 0x4320033e);
    }

    #[test]
    fn parse_hoptplug_ack() {
        let line = lines().nth(5).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/7:1");
        assert_eq!(entry.pid(), 164);
        assert_eq!(entry.cpu(), 7);
        assert_eq!(*entry.timestamp(), TimeVal::new(59, 265405));
        assert_eq!(entry.function(), "tb_tx");
        assert_eq!(entry.pdf(), Pdf::Notification);
        assert_eq!(entry.size(), 3);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 0);
        assert_eq!(entry.route(), 0);
        assert_eq!(entry.offset(), None);
        assert_eq!(entry.event(), Some(Event::HpAck));
        assert_eq!(entry.dwords(), None);
        assert_eq!(entry.adapter_num(), Some(5));
        assert_eq!(entry.cs(), None);
        assert_eq!(entry.sn(), None);
        let data = entry.data();
        assert_eq!(data.len(), 3);
        assert_eq!(data[0], 0x00000000);
        assert_eq!(data[1], 0x00000000);
        assert_eq!(data[2], 0x80000507);
    }

    #[test]
    fn parse_xdomain_response() {
        let line = lines().nth(8).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/12:1");
        assert_eq!(entry.pid(), 134);
        assert_eq!(entry.cpu(), 12);
        assert_eq!(*entry.timestamp(), TimeVal::new(5425, 790705));
        assert_eq!(entry.function(), "tb_tx");
        assert_eq!(entry.pdf(), Pdf::XdomainResponse);
        assert_eq!(entry.size(), 14);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 1);
        assert_eq!(entry.route(), 1);
        assert_eq!(entry.offset(), None);
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), None);
        assert_eq!(entry.adapter_num(), None);
        assert_eq!(entry.cs(), None);
        assert_eq!(entry.sn(), None);
        let data = entry.data();
        assert_eq!(data.len(), 14);
        assert_eq!(data[0], 0x00000000);
        assert_eq!(data[1], 0x00000001);
        assert_eq!(data[2], 0x0000000b);
        assert_eq!(data[3], 0x0ed738b6);
        assert_eq!(data[4], 0xbb40ff42);
        assert_eq!(data[5], 0xe290c297);
        assert_eq!(data[6], 0x07ffb2c0);
        assert_eq!(data[7], 0x00000002);
        assert_eq!(data[8], 0x80877f49);
        assert_eq!(data[9], 0x256a86f1);
        assert_eq!(data[10], 0xffffffff);
        assert_eq!(data[11], 0xffffffff);
        assert_eq!(data[12], 0x00000000);
        assert_eq!(data[13], 0x00000001);
        let packet = entry.packet();
        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert!(packet.is_xdomain());
        assert!(packet.uuid().is_some());
        assert_eq!(
            packet.uuid().unwrap().to_string(),
            "b638d70e-42ff-40bb-97c2-90e2c0b2ff07"
        );
        assert!(packet.field_by_bitfield_name("SN").is_some());
        assert_eq!(packet.field_by_bitfield_name("SN").unwrap().field("SN"), 0);
        assert_eq!(packet.data_size(), None);
        assert!(packet.packet_type().is_some());
        assert_eq!(packet.packet_type().unwrap(), (2, "UUID Response"));
        let fields = packet.fields();
        assert_eq!(fields.len(), 14);
        assert_eq!(fields[0].name().unwrap(), "Route String High");
        assert_eq!(fields[0].value(), 0);
        assert_eq!(fields[1].name().unwrap(), "Route String Low");
        assert_eq!(fields[1].value(), 1);
        assert_eq!(fields[3].name().unwrap(), "UUID");
        assert_eq!(fields[3].value(), 0x0ed738b6);
        assert_eq!(fields[4].name().unwrap(), "UUID");
        assert_eq!(fields[4].value(), 0xbb40ff42);
        assert_eq!(fields[5].name().unwrap(), "UUID");
        assert_eq!(fields[5].value(), 0xe290c297);
        assert_eq!(fields[6].name().unwrap(), "UUID");
        assert_eq!(fields[6].value(), 0x07ffb2c0);
        assert_eq!(fields[8].name().unwrap(), "Source UUID");
        assert_eq!(fields[8].value(), 0x80877f49);
        assert_eq!(fields[9].name().unwrap(), "Source UUID");
        assert_eq!(fields[9].value(), 0x256a86f1);
        assert_eq!(fields[10].name().unwrap(), "Source UUID");
        assert_eq!(fields[10].value(), 0xffffffff);
        assert_eq!(fields[11].name().unwrap(), "Source UUID");
        assert_eq!(fields[11].value(), 0xffffffff);
        let uuid: Vec<_> = fields[8..=11].iter().map(|f| f.value).collect();
        let uuid = util::u32_to_uuid(&uuid);
        assert!(uuid.is_some());
        assert_eq!(
            uuid.unwrap().to_string(),
            "497f8780-f186-6a25-ffff-ffffffffffff"
        );
    }

    #[test]
    fn parse_xdomain_request() {
        let line = lines().nth(9).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/u28:0");
        assert_eq!(entry.pid(), 11);
        assert_eq!(entry.cpu(), 2);
        assert_eq!(*entry.timestamp(), TimeVal::new(5425, 995295));
        assert_eq!(entry.function(), "tb_tx");
        assert_eq!(entry.pdf(), Pdf::XdomainRequest);
        assert_eq!(entry.size(), 8);
        assert_eq!(entry.dropped(), false);
        assert_eq!(entry.domain_index(), 1);
        assert_eq!(entry.route(), 1);
        assert_eq!(entry.offset(), None);
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), None);
        assert_eq!(entry.adapter_num(), None);
        assert_eq!(entry.cs(), None);
        assert_eq!(entry.sn(), None);
        let data = entry.data();
        assert_eq!(data.len(), 8);
        assert_eq!(data[0], 0x00000000);
        assert_eq!(data[1], 0x00000001);
        assert_eq!(data[2], 0x10000005);
        assert_eq!(data[3], 0x0ed738b6);
        assert_eq!(data[4], 0xbb40ff42);
        assert_eq!(data[5], 0xe290c297);
        assert_eq!(data[6], 0x07ffb2c0);
        assert_eq!(data[7], 0x0000000c);
        let packet = entry.packet();
        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert!(packet.is_xdomain());
        assert!(packet.uuid().is_some());
        assert_eq!(
            packet.uuid().unwrap().to_string(),
            "b638d70e-42ff-40bb-97c2-90e2c0b2ff07"
        );
        assert!(packet.field_by_bitfield_name("SN").is_some());
        assert_eq!(packet.field_by_bitfield_name("SN").unwrap().field("SN"), 2);
        assert_eq!(packet.data_size(), None);
        assert!(packet.packet_type().is_some());
        assert_eq!(packet.packet_type().unwrap(), (12, "UUID Request"));
        let fields = packet.fields();
        assert_eq!(fields.len(), 8);
        assert_eq!(fields[3].name().unwrap(), "UUID");
        assert_eq!(fields[3].value(), 0x0ed738b6);
        assert_eq!(fields[4].name().unwrap(), "UUID");
        assert_eq!(fields[4].value(), 0xbb40ff42);
        assert_eq!(fields[5].name().unwrap(), "UUID");
        assert_eq!(fields[5].value(), 0xe290c297);
        assert_eq!(fields[6].name().unwrap(), "UUID");
        assert_eq!(fields[6].value(), 0x07ffb2c0);
    }

    #[test]
    fn parse_usb4net_login() {
        let line = lines().nth(10).unwrap();
        let entry = Entry::parse(line);
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.task(), "kworker/12:1");
        assert_eq!(entry.pid(), 134);
        assert_eq!(entry.cpu(), 12);
        assert_eq!(*entry.timestamp(), TimeVal::new(5433, 028883));
        assert_eq!(entry.function(), "tb_rx");
        assert_eq!(entry.pdf(), Pdf::XdomainResponse);
        assert_eq!(entry.size(), 23);
        assert_eq!(entry.dropped(), true);
        assert_eq!(entry.domain_index(), 1);
        assert_eq!(entry.route(), 1);
        assert_eq!(entry.offset(), None);
        assert_eq!(entry.event(), None);
        assert_eq!(entry.dwords(), None);
        assert_eq!(entry.adapter_num(), None);
        assert_eq!(entry.cs(), None);
        assert_eq!(entry.sn(), None);
        let data = entry.data();
        assert_eq!(data.len(), 23);
        assert_eq!(data[0], 0x80000000);
        assert_eq!(data[1], 0x00000001);
        assert_eq!(data[2], 0x08000014);
        assert_eq!(data[3], 0x9e588f79);
        assert_eq!(data[4], 0x478a1636);
        assert_eq!(data[5], 0x6456c697);
        assert_eq!(data[6], 0xddc820a9);
        assert_eq!(data[7], 0x000003d7);
        assert_eq!(data[8], 0x186f7000);
        assert_eq!(data[9], 0x4a0d7aa3);
        assert_eq!(data[10], 0x1d925063);
        assert_eq!(data[11], 0x80877f49);
        assert_eq!(data[12], 0x256a86f1);
        assert_eq!(data[13], 0xffffffff);
        assert_eq!(data[14], 0xffffffff);
        assert_eq!(data[15], 0x00000000);
        assert_eq!(data[16], 0x00000002);
        assert_eq!(data[17], 0x00000001);
        assert_eq!(data[18], 0x00000008);
        assert_eq!(data[19], 0x00000000);
        assert_eq!(data[20], 0x00000000);
        assert_eq!(data[21], 0x00000000);
        assert_eq!(data[22], 0x00000000);
        let packet = entry.packet();
        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert!(packet.is_xdomain());
        assert!(packet.uuid().is_some());
        assert_eq!(
            packet.uuid().unwrap().to_string(),
            "798f589e-3616-8a47-97c6-5664a920c8dd"
        );
        assert_eq!(packet.route(), 0x80000000_00000001);
        assert!(packet.cm());
        assert!(packet.field_by_bitfield_name("SN").is_some());
        assert_eq!(packet.field_by_bitfield_name("SN").unwrap().field("SN"), 1);
        assert_eq!(packet.data_size(), None);
        assert!(packet.packet_type().is_some());
        assert_eq!(packet.packet_type().unwrap(), (0, "Login"));
        let fields = packet.fields();
        assert_eq!(fields.len(), 23);
        assert_eq!(fields[3].name().unwrap(), "UUID");
        assert_eq!(fields[3].value(), 0x9e588f79);
        assert_eq!(fields[4].name().unwrap(), "UUID");
        assert_eq!(fields[4].value(), 0x478a1636);
        assert_eq!(fields[5].name().unwrap(), "UUID");
        assert_eq!(fields[5].value(), 0x6456c697);
        assert_eq!(fields[6].name().unwrap(), "UUID");
        assert_eq!(fields[6].value(), 0xddc820a9);
        let uuid = packet.uuid_by_name("Requestor UUID");
        assert!(uuid.is_some());
        assert_eq!(
            uuid.unwrap().to_string(),
            "d7030000-0070-6f18-a37a-0d4a6350921d"
        );
        let uuid = packet.uuid_by_name("Responder UUID");
        assert!(uuid.is_some());
        assert_eq!(
            uuid.unwrap().to_string(),
            "497f8780-f186-6a25-ffff-ffffffffffff"
        );
        assert!(fields[16].has_field("Request ID"));
        assert_eq!(fields[16].field("Request ID"), 2);
        assert!(fields[17].has_field("USB4NET Service Revision"));
        assert_eq!(fields[17].field("USB4NET Service Revision"), 1);
        assert!(fields[18].has_field("Service Hop ID"));
        assert_eq!(fields[18].field("Service Hop ID"), 8);
        assert!(fields[19].name().is_none());
        assert!(fields[20].name().is_none());
        assert!(fields[21].name().is_none());
        assert!(fields[22].name().is_none());
    }
}
