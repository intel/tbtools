// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2024, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Implements router Device ROM (DROM) support.
//!
//! To be able to read DROM you need to have recent kernel. Support for this was added for v6.14.
//! # Examples
//! The below example shows how to iterate over the DROM generic entries and print the vendor and
//! model name.
//!
//! ```no_run
//! # use std::io;
//! # use tbtools::Address;
//! use tbtools::drom::{Drom, DromEntry};
//! # fn main() -> io::Result<()> {
//! # if let Some(mut device) = tbtools::find_device(&Address::Router { domain: 0, route: 1 })? {
//!
//! device.read_drom()?;
//!
//! if let Some(drom) = device.drom() {
//!     for entry in drom.entries().filter(|e| e.is_generic()) {
//!         match entry {
//!             DromEntry::AsciiVendorName(vendor) => println!("vendor: {}", vendor),
//!             DromEntry::AsciiModelName(model) => println!("model: {}", model),
//!             _ => (),
//!         }
//!     }
//! }
//! # }
//! # Ok(())
//! # }

use crate::{
    debugfs::{Adapter, Type},
    genmask_t, util, Version,
};
use std::{
    ffi::CStr,
    io::{Error, ErrorKind, Result},
};

const DROM_LENGTH_MASK: u16 = genmask_t!(u16, 11, 0);

const DROM_AE: u8 = 1 << 7;
const DROM_AD: u8 = 1 << 6;
const DROM_AN_MASK: u8 = genmask_t!(u8, 5, 0);

const DROM_KIND_MASK: u8 = genmask_t!(u8, 5, 0);
const DROM_KIND_ASCII_VENDOR_NAME: u8 = 0x1;
const DROM_KIND_ASCII_MODEL_NAME: u8 = 0x2;
const DROM_KIND_TMU: u8 = 0x8;
const DROM_KIND_PRODUCT: u8 = 0x9;
const DROM_KIND_SERIAL_NUMBER: u8 = 0xa;
const DROM_KIND_USB3_PORT_MAPPING: u8 = 0xb;
const DROM_KIND_UTF16_VENDOR_NAME: u8 = 0xc;
const DROM_KIND_UTF16_MODEL_NAME: u8 = 0xd;

const DROM_DP_PV: u8 = 1 << 6;
const DROM_DP_PA_MASK: u8 = genmask_t!(u8, 5, 0);

const DROM_LANE_DLC: u8 = 1 << 7;
const DROM_LANE_L1A: u8 = 1 << 4;
const DROM_LANE_DLA_MASK: u8 = genmask_t!(u8, 5, 0);

const DROM_PCIE_FN_MASK: u8 = genmask_t!(u8, 2, 0);
const DROM_PCIE_DEV_HI_MASK: u8 = genmask_t!(u8, 4, 3);
const DROM_PCIE_DEV_HI_SHIFT: u8 = 3;
const DROM_PCIE_DEV_LO_MASK: u8 = genmask_t!(u8, 7, 5);
const DROM_PCIE_DEV_LO_SHIFT: u8 = 5;

const DROM_TMU_MODE_MASK: u8 = genmask_t!(u8, 1, 0);
const DROM_TMU_MODE_OFF: u8 = 0x0;
const DROM_TMU_MODE_UNI: u8 = 0x1;
const DROM_TMU_MODE_BI: u8 = 0x2;

const DROM_TMU_RATE_MASK: u8 = genmask_t!(u8, 3, 2);
const DROM_TMU_RATE_SHIFT: u8 = 2;
const DROM_TMU_RATE_HIFI: u8 = 0x1;
const DROM_TMU_MODE_LOWRES: u8 = 0x2;

/// Possible modes of [`Tmu`](DromEntry::Tmu) generic entry.
#[derive(Clone, Debug, PartialEq)]
pub enum TmuMode {
    Unknown = -1,
    /// Preferred TMU mode is Off.
    Off,
    /// Preferred TMU mode is Unidirectional.
    Unidirectional,
    /// Preferred TMU mode is Bidirectional.
    Bidirectional,
}

/// Possible rates of [`Tmu`](DromEntry::Tmu) generic entry.
#[derive(Clone, Debug, PartialEq)]
pub enum TmuRate {
    Unknown,
    /// Preferred TMU refresh rate is HiFi.
    HiFi = 1,
    /// Preferred TMU refresh rate is LowRes.
    LowRes,
}

/// USB Port Mapping generic entry.
///
/// Required for USB4 hubs and standalone add-in-card USB4 hosts.
#[derive(Clone, Debug, PartialEq)]
pub struct Usb3PortMap {
    /// Downstream USB 3 port number of the internal SuperSpeed Plus host or hub.
    pub usb3_port_num: u8,
    /// USB Type-C connector number the USB 3 port is connected to (only valid if `usb_type_c` is
    /// `true`.
    pub pd_port_num: u8,
    /// Index number of the internal SuperSpeed Plus host controller.
    pub xhci_index: u8,
    /// Set to `true` if USB 3 port is connected to USB Type-C connector.
    pub usb_type_c: bool,
    /// USB 3 adapter number that is connected to the USB 3 port (only valid if `tunneling` is
    /// `true`.
    pub usb3_adapter_num: u8,
    /// Set to `true` if USB 3 port is connected to USB 3 adapter.
    pub tunneling: bool,
}

/// All known entry types.
#[derive(Clone, Debug, PartialEq)]
pub enum DromEntry<'a> {
    /// Entry is not known.
    Unknown(&'a [u8]),
    /// Uknnown adapter entry.
    Adapter {
        /// Is the adapter disabled.
        disabled: bool,
        /// Number of the adapter.
        adapter_num: u8,
    },
    /// Unused adapter entry.
    UnusedAdapter {
        /// Number of the disabled adapter.
        adapter_num: u8,
    },
    /// DisplayPort adapter entry.
    DisplayPortAdapter {
        /// Number of the DisplayPort adapter.
        adapter_num: u8,
        /// Lane adapter number that is preferred when setting up a path to this DisplayPort
        /// adapter. Only valid when `preference_valid` is `true`.
        preferred_lane_adapter_num: u8,
        /// Set to `true` if there is preference for a certain lane adapter.
        preference_valid: bool,
    },
    /// Thunderbolt 3 lane adapter entry.
    LaneAdapter {
        /// Lane adapter number.
        adapter_num: u8,
        /// Set to `true` if this is lane1 adapter.
        lane_1_adapter: bool,
        /// Set to `true` if this is is USB4 port has two lanes.
        dual_lane_adapter: bool,
        /// Adapter number of the other lane if `dual_lane_adapter` is `true`.
        dual_lane_adapter_num: u8,
    },
    /// Thunderbolt 3 PCIe upstream adapter entry.
    PcieUpAdapter {
        /// PCIe upstream adapter number.
        adapter_num: u8,
        /// Function number that associates the adapter to PCIe switch/endpoint.
        function_num: u8,
        /// PCIe device number that associates this adapter to PCIe switch/endpoint.
        device_num: u8,
    },
    /// Thunderbolt 3 PCIe downstream adapter entry.
    PcieDownAdapter {
        /// PCIe downstream adapter number.
        adapter_num: u8,
        /// Function number that associates the adapter to PCIe switch/endpoint.
        function_num: u8,
        /// PCIe device number that associates this adapter to PCIe switch/endpoint.
        device_num: u8,
    },
    /// Generic entry that is not known.
    Generic {
        /// Size of the entry in bytes.
        length: usize,
        /// Type of the entry.
        kind: u8,
        /// Raw bytes of the entry.
        data: &'a [u8],
    },
    /// ACII vendor name generic entry.
    AsciiVendorName(&'a str),
    /// ACII model name generic entry.
    AsciiModelName(&'a str),
    /// TMU minimum requirements generic entry.
    Tmu { mode: TmuMode, rate: TmuRate },
    /// USB4 product descriptor generic entry.
    ProductDescriptor {
        /// USB4 specification number.
        usb4_version: Version,
        /// Product Vendor ID (VID)
        vendor: u16,
        /// Product ID (PID)
        product: u16,
        /// Product firmwware version.
        fw_version: Version,
        /// Product Test ID (TID).
        test_id: u32,
        /// Product hardware revision.
        hw_revision: u8,
    },
    /// Serial number generic entry.
    SerialNumber { lang_id: u16, serial_number: String },
    /// USB port mapping generic entry.
    Usb3PortMapping(Vec<Usb3PortMap>),
    /// UTF-16 vendor name generic entry.
    Utf16VendorName(String),
    /// UTF-16 model name generic entry.
    Utf16ModelName(String),
}

impl<'a> DromEntry<'a> {
    /// Returns `true` if this is adapter entry.
    pub fn is_adapter(&self) -> bool {
        matches!(
            *self,
            Self::Adapter { .. }
                | Self::UnusedAdapter { .. }
                | Self::DisplayPortAdapter { .. }
                | Self::LaneAdapter { .. }
                | Self::PcieUpAdapter { .. }
                | Self::PcieDownAdapter { .. }
        )
    }

    /// Returns `true` if this is generic entry.
    pub fn is_generic(&self) -> bool {
        !self.is_adapter()
    }

    fn parse(bytes: &'a [u8], adapters: &[Adapter]) -> Self {
        if bytes.len() < 2 {
            return Self::Unknown(bytes);
        }

        let length: usize = bytes[0].into();

        if (bytes[1] & DROM_AE) > 0 {
            if (bytes[1] & DROM_AD) > 0 {
                Self::UnusedAdapter {
                    adapter_num: bytes[1] & DROM_AN_MASK,
                }
            } else {
                let adapter_num = bytes[1] & DROM_AN_MASK;

                if let Some(adapter) = adapters.iter().find(|a| a.adapter() == adapter_num) {
                    match adapter.kind() {
                        Type::DisplayPortIn | Type::DisplayPortOut => Self::DisplayPortAdapter {
                            adapter_num,
                            preferred_lane_adapter_num: bytes[4] & DROM_DP_PA_MASK,
                            preference_valid: bytes[4] & DROM_DP_PV > 0,
                        },

                        // Thunderbolt 3 compatible.
                        Type::Lane => Self::LaneAdapter {
                            adapter_num,
                            lane_1_adapter: bytes[2] & DROM_LANE_L1A > 0,
                            dual_lane_adapter: bytes[2] & DROM_LANE_DLC > 0,
                            dual_lane_adapter_num: bytes[3] & DROM_LANE_DLA_MASK,
                        },

                        Type::PcieUp => {
                            let hi = (bytes[2] & DROM_PCIE_DEV_HI_MASK) >> DROM_PCIE_DEV_HI_SHIFT;
                            let lo = (bytes[2] & DROM_PCIE_DEV_LO_MASK) >> DROM_PCIE_DEV_LO_SHIFT;
                            let device = hi << 3 | lo;

                            Self::PcieUpAdapter {
                                adapter_num,
                                function_num: bytes[2] & DROM_PCIE_FN_MASK,
                                device_num: device,
                            }
                        }

                        Type::PcieDown => {
                            let hi = (bytes[2] & DROM_PCIE_DEV_HI_MASK) >> DROM_PCIE_DEV_HI_SHIFT;
                            let lo = (bytes[2] & DROM_PCIE_DEV_LO_MASK) >> DROM_PCIE_DEV_LO_SHIFT;
                            let device = hi << 3 | lo;

                            Self::PcieDownAdapter {
                                adapter_num,
                                function_num: bytes[2] & DROM_PCIE_FN_MASK,
                                device_num: device,
                            }
                        }

                        _ => Self::Adapter {
                            disabled: (bytes[1] & DROM_AD) > 0,
                            adapter_num: bytes[1] & DROM_AN_MASK,
                        },
                    }
                } else {
                    Self::Adapter {
                        disabled: (bytes[1] & DROM_AD) > 0,
                        adapter_num,
                    }
                }
            }
        } else {
            let kind = bytes[1] & DROM_KIND_MASK;

            match kind {
                DROM_KIND_ASCII_VENDOR_NAME => {
                    let cstr = CStr::from_bytes_until_nul(&bytes[2..]).unwrap();
                    Self::AsciiVendorName(cstr.to_str().unwrap())
                }

                DROM_KIND_ASCII_MODEL_NAME => {
                    let cstr = CStr::from_bytes_until_nul(&bytes[2..]).unwrap();
                    Self::AsciiModelName(cstr.to_str().unwrap())
                }

                DROM_KIND_TMU => Self::Tmu {
                    mode: match bytes[2] & DROM_TMU_MODE_MASK {
                        DROM_TMU_MODE_OFF => TmuMode::Off,
                        DROM_TMU_MODE_UNI => TmuMode::Unidirectional,
                        DROM_TMU_MODE_BI => TmuMode::Bidirectional,
                        _ => TmuMode::Unknown,
                    },
                    rate: match (bytes[2] & DROM_TMU_RATE_MASK) >> DROM_TMU_RATE_SHIFT {
                        DROM_TMU_RATE_HIFI => TmuRate::HiFi,
                        DROM_TMU_MODE_LOWRES => TmuRate::LowRes,
                        _ => TmuRate::Unknown,
                    },
                },

                DROM_KIND_PRODUCT => Self::ProductDescriptor {
                    usb4_version: Version {
                        major: bytes[3],
                        minor: bytes[2],
                    },
                    vendor: u16::from_le_bytes(<[u8; 2]>::try_from(&bytes[4..=5]).unwrap()),
                    product: u16::from_le_bytes(<[u8; 2]>::try_from(&bytes[6..=7]).unwrap()),
                    fw_version: Version {
                        major: bytes[9],
                        minor: bytes[8],
                    },
                    test_id: u32::from_le_bytes(<[u8; 4]>::try_from(&bytes[10..=13]).unwrap()),
                    hw_revision: bytes[10],
                },

                DROM_KIND_SERIAL_NUMBER => Self::SerialNumber {
                    lang_id: u16::from_le_bytes(<[u8; 2]>::try_from(&bytes[2..=3]).unwrap()),
                    serial_number: util::bytes_to_utf16_ascii(&bytes[4..]),
                },

                DROM_KIND_USB3_PORT_MAPPING => {
                    let mut mappings = Vec::new();

                    bytes[2..].chunks_exact(3).for_each(|m| {
                        let mapping = Usb3PortMap {
                            usb3_port_num: m[0] & 0xf,
                            pd_port_num: m[1] & 0x1f,
                            xhci_index: (m[1] & 0x60) >> 5,
                            usb_type_c: m[1] & 0x80 > 0,
                            usb3_adapter_num: m[2] & 0x3f,
                            tunneling: m[2] & 0x80 > 0,
                        };
                        mappings.push(mapping);
                    });

                    Self::Usb3PortMapping(mappings)
                }

                DROM_KIND_UTF16_VENDOR_NAME => {
                    Self::Utf16VendorName(util::bytes_to_utf16_ascii(&bytes[2..]))
                }

                DROM_KIND_UTF16_MODEL_NAME => {
                    Self::Utf16ModelName(util::bytes_to_utf16_ascii(&bytes[2..]))
                }

                _ => Self::Generic {
                    length,
                    kind,
                    data: &bytes[2..],
                },
            }
        }
    }
}

/// An iterator over adapter and generic entries of the DROM.
pub struct DromEntries<'a> {
    start: usize,
    offset: usize,
    len: usize,
    bytes: &'a [u8],
    adapters: &'a [Adapter],
}

impl<'a> DromEntries<'a> {
    fn new(start: usize, bytes: &'a [u8], adapters: &'a [Adapter]) -> Self {
        Self {
            start,
            offset: start,
            len: bytes[start].into(),
            bytes,
            adapters,
        }
    }

    /// Raw bytes of the current DROM entry.
    pub fn bytes(&self) -> &'a [u8] {
        &self.bytes[self.start()..=self.end()]
    }

    /// Starting offset of the current DROM entry.
    pub fn start(&self) -> usize {
        self.start
    }

    /// Ending offset of the current DROM entry.
    pub fn end(&self) -> usize {
        self.start + self.len - 1
    }

    /// Length in bytes of the current DROM entry.
    pub fn length(&self) -> usize {
        self.len
    }
}

impl<'a> Iterator for DromEntries<'a> {
    type Item = DromEntry<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.bytes.len() {
            return None;
        }

        self.start = self.offset;
        self.len = self.bytes[self.start].into();

        let entry = DromEntry::parse(self.bytes(), self.adapters);

        self.offset += self.len;

        Some(entry)
    }
}

/// Device ROM (DROM) structure.
#[derive(Clone, Debug)]
pub struct Drom {
    bytes: Vec<u8>,
    adapters: Vec<Adapter>,
    crc32: u32,
    length: usize,
    start: usize,
    version: u8,
}

impl Drom {
    /// Returns CRC8 field calculated over UUID.
    ///
    /// Only when [`is_tb3_compatible`](Self::is_tb3_compatible) returns `true`.
    pub fn crc8(&self) -> Option<u8> {
        if self.is_tb3_compatible() {
            Some(self.bytes[0])
        } else {
            None
        }
    }

    /// Returns `true` if the CRC8 is matches the actual.
    ///
    /// Only when [`is_tb3_compatible`](Self::is_tb3_compatible) returns `true`.
    pub fn is_crc8_valid(&self) -> bool {
        if self.is_tb3_compatible() {
            let crc8 = util::crc8(&self.bytes[1..=8]);
            crc8 == self.crc8().unwrap()
        } else {
            false
        }
    }

    /// Returns UUID.
    ///
    /// Only when [`is_tb3_compatible`](Self::is_tb3_compatible) returns `true`.
    pub fn uuid(&self) -> Option<u64> {
        if self.is_tb3_compatible() {
            Some(u64::from_le_bytes(
                <[u8; 8]>::try_from(&self.bytes[1..=8]).unwrap(),
            ))
        } else {
            None
        }
    }

    /// Returns raw bytes of the header part of the DROM.
    pub fn header(&self) -> &[u8] {
        &self.bytes[..self.start]
    }

    /// Returns raw bytes of the body of the DROM.
    pub fn body(&self) -> &[u8] {
        &self.bytes[self.start..]
    }

    /// Returns the raw bytes of the whole DROM.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns CRC32 calculated over the DROM.
    pub fn crc32(&self) -> u32 {
        self.crc32
    }

    /// Returns `true` if the CRC32 is matches the actual.
    pub fn is_crc32_valid(&self) -> bool {
        let crc32 = util::crc32(&self.bytes[13..]);
        crc32 == self.crc32
    }

    /// Returns USB4 DROM specification version number.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// Returns size of the DROM starting from `version` field.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Returns `true` if this DROM is Thunderbolt 3 compatible.
    pub fn is_tb3_compatible(&self) -> bool {
        self.version() < 3
    }

    /// Produces an iterator over the [`DromEntries`] of the DROM.
    pub fn entries(&self) -> DromEntries<'_> {
        DromEntries::new(self.start, &self.bytes, &self.adapters)
    }

    pub(crate) fn parse(bytes: &[u8], adapters: &[Adapter]) -> Result<Self> {
        if bytes.len() < 16 {
            return Err(Error::new(ErrorKind::InvalidData, "DROM too small"));
        }

        let crc32 = u32::from_le_bytes(<[u8; 4]>::try_from(&bytes[9..=12]).unwrap());
        let length = u16::from_le_bytes(<[u8; 2]>::try_from(&bytes[14..=15]).unwrap());
        let length: usize = (length & DROM_LENGTH_MASK).into();

        // Sanity check.
        if length + 13 != bytes.len() {
            return Err(Error::new(ErrorKind::InvalidData, "DROM size mismatch"));
        }

        let version = bytes[13];
        let start: usize = if version < 3 {
            // Make sure whole TBT 3 header section is covered.
            if bytes.len() < 22 {
                return Err(Error::new(ErrorKind::InvalidData, "DROM size mismatch"));
            }
            22
        } else {
            16
        };

        Ok(Self {
            bytes: Vec::from(bytes),
            adapters: Vec::from(adapters),
            crc32,
            length,
            version,
            start,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    const HOST_DROM: [u8; 60] = [
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0xe7, 0xa7, 0x7c, 0x03, 0x2f,
        0x00, 0x05, 0x85, 0x00, 0x00, 0x41, 0x05, 0x86, 0x00, 0x00, 0x43, 0x08, 0x01, 0x49, 0x6e,
        0x74, 0x65, 0x6c, 0x00, 0x08, 0x02, 0x47, 0x65, 0x6e, 0x31, 0x34, 0x00, 0x03, 0x08, 0x00,
        0x0f, 0x09, 0x10, 0x04, 0x87, 0x80, 0xb2, 0x7e, 0x01, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    const DEVICE_DROM: [u8; 231] = [
        0x87, 0x00, 0x84, 0xe9, 0x78, 0xf8, 0x2f, 0x87, 0x80, 0x9e, 0x80, 0xd1, 0x89, 0x01, 0xda,
        0x00, 0x87, 0x80, 0x34, 0x12, 0x03, 0x02, 0x08, 0x81, 0x80, 0x02, 0x00, 0x00, 0x00, 0x00,
        0x08, 0x82, 0x90, 0x01, 0x00, 0x00, 0x00, 0x00, 0x08, 0x83, 0x80, 0x04, 0x00, 0x00, 0x00,
        0x00, 0x08, 0x84, 0x90, 0x03, 0x00, 0x00, 0x00, 0x00, 0x08, 0x85, 0x80, 0x06, 0x00, 0x00,
        0x00, 0x00, 0x08, 0x86, 0x90, 0x05, 0x00, 0x00, 0x00, 0x00, 0x08, 0x87, 0x80, 0x08, 0x00,
        0x00, 0x00, 0x00, 0x08, 0x88, 0x90, 0x07, 0x00, 0x00, 0x00, 0x00, 0x0b, 0x89, 0x20, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x8a, 0x20, 0x03, 0x8b, 0x40, 0x03, 0x8c,
        0x60, 0x05, 0x8d, 0x00, 0x00, 0x41, 0x05, 0x8e, 0x00, 0x00, 0x43, 0x28, 0x01, 0x49, 0x6e,
        0x74, 0x65, 0x6c, 0x20, 0x54, 0x68, 0x75, 0x6e, 0x64, 0x65, 0x72, 0x62, 0x6f, 0x6c, 0x74,
        0x20, 0x67, 0x65, 0x6e, 0x65, 0x72, 0x69, 0x63, 0x20, 0x76, 0x65, 0x6e, 0x64, 0x6f, 0x72,
        0x20, 0x6e, 0x61, 0x6d, 0x65, 0x00, 0x27, 0x02, 0x49, 0x6e, 0x74, 0x65, 0x6c, 0x20, 0x54,
        0x68, 0x75, 0x6e, 0x64, 0x65, 0x72, 0x62, 0x6f, 0x6c, 0x74, 0x20, 0x67, 0x65, 0x6e, 0x65,
        0x72, 0x69, 0x63, 0x20, 0x6d, 0x6f, 0x64, 0x65, 0x6c, 0x20, 0x6e, 0x61, 0x6d, 0x65, 0x00,
        0x03, 0x08, 0x09, 0x0f, 0x09, 0x10, 0x04, 0x87, 0x80, 0x09, 0x00, 0x02, 0x43, 0x00, 0x00,
        0x00, 0x00, 0x03, 0x0e, 0x0b, 0x01, 0x81, 0x91, 0x02, 0x82, 0x92, 0x03, 0x83, 0x93, 0x04,
        0x00, 0x00, 0x04, 0x30, 0x01, 0x57,
    ];

    #[test]
    fn parse_host_drom() {
        let adapters = vec![
            Adapter::new(5, Type::DisplayPortIn, None, true, false),
            Adapter::new(6, Type::DisplayPortIn, None, true, false),
        ];
        let drom = Drom::parse(&HOST_DROM, &adapters);
        assert!(drom.is_ok());
        let drom = drom.unwrap();
        assert_eq!(drom.crc8(), None);
        assert_eq!(drom.crc32(), 0x7ca7e730);
        assert_eq!(drom.is_crc32_valid(), true);
        assert_eq!(drom.version(), 3);
        assert_eq!(drom.is_tb3_compatible(), false);
        assert_eq!(drom.length(), 47);
        let mut entries = drom.entries();
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::DisplayPortAdapter {
                adapter_num: 5,
                preferred_lane_adapter_num: 1,
                preference_valid: true,
            }
        );
        assert_eq!(entries.start(), 16);
        assert_eq!(entries.length(), 5);
        assert_eq!(entries.bytes(), [0x5, 0x85, 0x0, 0x0, 0x41]);
        assert_eq!(entries.bytes(), &HOST_DROM[entries.start()..=entries.end()]);
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::DisplayPortAdapter {
                adapter_num: 6,
                preferred_lane_adapter_num: 3,
                preference_valid: true,
            }
        );
        assert_eq!(entries.next().unwrap(), DromEntry::AsciiVendorName("Intel"));
        assert_eq!(entries.next().unwrap(), DromEntry::AsciiModelName("Gen14"));
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::Tmu {
                mode: TmuMode::Off,
                rate: TmuRate::Unknown,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::ProductDescriptor {
                usb4_version: Version {
                    major: 0x4,
                    minor: 0x10,
                },
                vendor: 0x8087,
                product: 0x7eb2,
                fw_version: Version { major: 9, minor: 1 },
                test_id: 0,
                hw_revision: 0,
            }
        );
        assert_eq!(entries.start(), 45);
        assert_eq!(entries.end(), 59);
        assert_eq!(entries.length(), 15);
        assert_eq!(
            entries.bytes(),
            [0xf, 0x9, 0x10, 0x4, 0x87, 0x80, 0xb2, 0x7e, 1, 9, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn parse_device_drom() {
        let adapters = vec![
            Adapter::new(1, Type::Lane, None, true, false),
            Adapter::new(2, Type::Lane, None, true, false),
            Adapter::new(3, Type::Lane, None, true, false),
            Adapter::new(4, Type::Lane, None, true, false),
            Adapter::new(5, Type::Lane, None, true, false),
            Adapter::new(6, Type::Lane, None, true, false),
            Adapter::new(7, Type::Lane, None, true, false),
            Adapter::new(8, Type::Lane, None, true, false),
            Adapter::new(9, Type::PcieUp, None, true, false),
            Adapter::new(10, Type::PcieDown, None, true, false),
            Adapter::new(11, Type::PcieDown, None, true, false),
            Adapter::new(12, Type::PcieDown, None, true, false),
            Adapter::new(13, Type::DisplayPortOut, None, true, false),
            Adapter::new(14, Type::DisplayPortOut, None, true, false),
        ];
        let drom = Drom::parse(&DEVICE_DROM, &adapters);
        assert!(drom.is_ok());
        let drom = drom.unwrap();
        assert_eq!(drom.crc8(), Some(0x87));
        assert_eq!(drom.is_crc8_valid(), true);
        assert_eq!(drom.crc32(), 0x89d1809e);
        assert_eq!(drom.is_crc32_valid(), true);
        assert_eq!(drom.version(), 1);
        assert_eq!(drom.is_tb3_compatible(), true);
        assert_eq!(drom.length(), 218);
        let mut entries = drom.entries();
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 1,
                lane_1_adapter: false,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 2,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 2,
                lane_1_adapter: true,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 1,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 3,
                lane_1_adapter: false,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 4,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 4,
                lane_1_adapter: true,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 3,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 5,
                lane_1_adapter: false,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 6,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 6,
                lane_1_adapter: true,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 5,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 7,
                lane_1_adapter: false,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 8,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::LaneAdapter {
                adapter_num: 8,
                lane_1_adapter: true,
                dual_lane_adapter: true,
                dual_lane_adapter_num: 7,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::PcieUpAdapter {
                adapter_num: 9,
                function_num: 0,
                device_num: 0x1,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::PcieDownAdapter {
                adapter_num: 10,
                function_num: 0,
                device_num: 0x1,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::PcieDownAdapter {
                adapter_num: 11,
                function_num: 0,
                device_num: 0x2,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::PcieDownAdapter {
                adapter_num: 12,
                function_num: 0,
                device_num: 0x3,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::DisplayPortAdapter {
                adapter_num: 13,
                preferred_lane_adapter_num: 1,
                preference_valid: true,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::DisplayPortAdapter {
                adapter_num: 14,
                preferred_lane_adapter_num: 3,
                preference_valid: true,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::AsciiVendorName("Intel Thunderbolt generic vendor name")
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::AsciiModelName("Intel Thunderbolt generic model name")
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::Tmu {
                mode: TmuMode::Unidirectional,
                rate: TmuRate::LowRes,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::ProductDescriptor {
                usb4_version: Version {
                    major: 0x4,
                    minor: 0x10,
                },
                vendor: 0x8087,
                product: 9,
                fw_version: Version {
                    major: 67,
                    minor: 2
                },
                test_id: 0,
                hw_revision: 0,
            }
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::Usb3PortMapping(vec![
                Usb3PortMap {
                    usb3_port_num: 1,
                    pd_port_num: 1,
                    xhci_index: 0,
                    usb_type_c: true,
                    usb3_adapter_num: 17,
                    tunneling: true,
                },
                Usb3PortMap {
                    usb3_port_num: 2,
                    pd_port_num: 2,
                    xhci_index: 0,
                    usb_type_c: true,
                    usb3_adapter_num: 18,
                    tunneling: true,
                },
                Usb3PortMap {
                    usb3_port_num: 3,
                    pd_port_num: 3,
                    xhci_index: 0,
                    usb_type_c: true,
                    usb3_adapter_num: 19,
                    tunneling: true,
                },
                Usb3PortMap {
                    usb3_port_num: 4,
                    pd_port_num: 0,
                    xhci_index: 0,
                    usb_type_c: false,
                    usb3_adapter_num: 0,
                    tunneling: false,
                }
            ])
        );
        assert_eq!(
            entries.next().unwrap(),
            DromEntry::Generic {
                length: 4,
                kind: 0x30,
                data: &[0x1, 0x57],
            }
        );
    }
}
