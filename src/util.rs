// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//!Miscellaneous utility functions.

use std::{
    char, fs,
    io::{self, Error, ErrorKind},
    str::FromStr,
};

use lazy_static::lazy_static;
use nix::{
    sys::time::{self, TimeVal},
    time::{clock_gettime, ClockId},
};
use num_traits::Num;
use regex::Regex;
use uuid;

/// Similar to kernel's `GENMASK()` macro.
///
/// # Examples
/// ```
/// use tbtools::genmask_t;
///
/// const TMU_RTR_CS_0_FREQ_WINDOW_MASK: u32 = genmask_t!(u32, 26, 16);
/// const TMU_RTR_CS_3_TS_PACKET_INTERVAL_MASK: u32 = genmask_t!(u32, 31, 16);
/// ```
#[macro_export]
macro_rules! genmask_t {
    ($t:ty, $high:expr, $low:expr) => {{
        <$t>::MAX - (1 << $low) + 1 & (<$t>::MAX >> (<$t>::BITS - 1 - $high))
    }};
}

/// Parse hexadecimal from string.
///
/// Assumes the string is hexadecimal and converts it to a number if possible, or `None` if no such
/// conversion is possible.
///
/// # Examples
/// ```
/// use tbtools::util;
///
/// if let Some(number) = util::parse_hex::<u32>("0x1234") {
///     assert_eq!(number, 0x1234);
/// }
/// ```
pub fn parse_hex<T: Num + FromStr>(s: &str) -> Option<T> {
    let val = match s.strip_prefix("0x") {
        Some(s) => s,
        None => s,
    };

    <T>::from_str_radix(val, 16).ok()
}

/// Parse route string from input string.
///
/// The input string should be hexadecimal route string. Returns the corresponding route as binary
/// or [`Err`] if parsing failed.
///
/// # Examples
/// ```
/// use tbtools::util;
///
/// if let Ok(route) = util::parse_route("701") {
///     assert_eq!(route, 0x701);
/// }
/// ```
pub fn parse_route(s: &str) -> Result<u64, String> {
    if let Some(route) = parse_hex::<u64>(s) {
        Ok(route)
    } else {
        Err(String::from("Invalid Route"))
    }
}

/// Parse any number hexadecimal or not.
///
/// Parses numeric string into binary regardless whether it is in hexadecimal format or not. If
/// conversion is not possible returns `None`.
/// # Examples
/// ```
/// use tbtools::util;
///
/// if let Some(number) = util::parse_number::<i32>("1234") {
///     assert_eq!(number, 1234);
/// }
/// ```
pub fn parse_number<T: Num + FromStr>(s: &str) -> Option<T> {
    // Try to match decimal digits first and if that matches use standard
    // functions to parse it.
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^\d+$").unwrap();
    }
    if RE.is_match(s) {
        return s.parse::<T>().ok();
    }
    parse_hex(s)
}

/// Converts [`u32`] array into [`Uuid`](uuid::Uuid).
///
/// This is useful when you have Inter-Domain packet with UUIDs represented as 4 u32.
pub fn u32_to_uuid(uuid: &[u32]) -> Option<uuid::Uuid> {
    if uuid.len() < 4 {
        return None;
    }

    // There must be an easier way for doing this.
    let mut bytes = Vec::new();
    for u in uuid {
        for b in u.to_le_bytes() {
            bytes.push(b);
        }
    }

    Some(uuid::Builder::from_slice(&bytes).ok()?.into_uuid())
}

/// Converts an array of bytes into printable ASCII.
///
/// # Examples
/// ```
/// use tbtools::util;
///
/// let ascii = util::bytes_to_ascii(&[0x64, 0x65, 0x76, 0x69]);
/// assert_eq!(ascii, "devi");
/// ```
pub fn bytes_to_ascii(bytes: &[u8]) -> String {
    let mut s = String::new();

    bytes.iter().for_each(|b| {
        if b.is_ascii_graphic() {
            s.push_str(&format!("{}", *b as char));
        } else {
            s.push('.')
        }
    });

    s
}

/// Converts an array of `u16` bytes into ASCII
///
/// # Panics
/// If `bytes` length is not multiple of `u16`.
pub fn bytes_to_utf16_ascii(bytes: &[u8]) -> String {
    let mut s = String::new();

    if bytes.len() % 2 != 0 {
        panic!("bytes not aligned by 16-bits");
    }

    let chunks = bytes
        .chunks(2)
        .map(|b| u16::from_le_bytes(<[u8; 2]>::try_from(b).unwrap()));

    char::decode_utf16(chunks).for_each(|b| {
        if let Ok(b) = b {
            if b.is_ascii_graphic() {
                s.push_str(&format!("{b}"));
            }
        } else {
            s.push('.')
        }
    });

    s
}

/// Returns system boot time in wall clock time.
pub fn system_boot_time() -> io::Result<TimeVal> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"^btime\s+(\d+)$").unwrap();
    }

    let stat = fs::read_to_string("/proc/stat")?;
    let btime: Vec<_> = stat.split('\n').filter(|s| RE.is_match(s)).collect();

    if btime.len() == 1 {
        if let Some(caps) = RE.captures(btime[0]) {
            if let Some(seconds) = parse_number::<time::time_t>(&caps[1]) {
                return Ok(TimeVal::new(seconds, 0));
            }
        }
    }

    Err(Error::from(ErrorKind::Unsupported))
}

/// Returns timestamp of now since system boot.
pub fn system_current_timestamp() -> TimeVal {
    let current_timestamp =
        clock_gettime(ClockId::CLOCK_MONOTONIC).expect("Failed to get current system time");

    TimeVal::new(
        current_timestamp.tv_sec(),
        current_timestamp.tv_nsec() / 1000,
    )
}

/// Define a single bit within a register
///
/// This type provides a compile time representation of how a given bit within
/// a register is to be parsed. The `get_bit()` and `set_bit()` methods operate
/// on an array slice that represents multiple consecutive words.
///
/// # Const Parameters
///
/// * `DWORD_OFFSET` - The index of the array of double words at which the bit resides
/// * `BIT` - The bit offset within the double word
///
/// # Examples
/// ```
/// use tbtools::util;
/// type ModesSW = util::RegBit<0, 1>; // Word offset 0, bit offset 1
///
/// let mut raw = [0u32; 2];
/// ModesSW::set_bit(&mut raw, true);
/// assert_eq!(raw[0], 1 << 1);
/// assert!(ModesSW::get_bit(&raw));
/// ```
pub struct RegBit<const DWORD_OFFSET: usize, const BIT: u32>;

impl<const DWORD_OFFSET: usize, const BIT: u32> RegBit<DWORD_OFFSET, BIT> {
    const MASK: u32 = 1u32 << BIT;
    const SHIFT: u32 = BIT;

    pub fn get_bit(raw: &[u32]) -> bool {
        (raw[DWORD_OFFSET] & Self::MASK) >> Self::SHIFT != 0
    }

    pub fn set_bit(raw: &mut [u32], value: bool) {
        raw[DWORD_OFFSET] = (!Self::MASK & raw[DWORD_OFFSET]) | if value { Self::MASK } else { 0 };
    }
}

/// Define a field within a register
///
/// See `RegBit` documentation for background.
///
/// # Const Parameters
///
/// * `DWORD_OFFSET` - The index of the array of double words at which the bit resides
/// * `LOW` - The bit offset of the lowest bit of the field
/// * `HIGH` - The bit offset of the highest bit of the field
///
/// # Examples
/// ```
/// use tbtools::util;
/// type VoltageIndp = util::RegField<1, 4, 3>;
/// const VOLTAGE_HL: u32 = 1;
///
/// let mut raw = [0u32; 2];
/// VoltageIndp::set_field(&mut raw, VOLTAGE_HL);
/// assert_eq!(raw[1], VOLTAGE_HL << 3);
/// assert_eq!(VoltageIndp::get_field(&raw), VOLTAGE_HL);
/// ```
pub struct RegField<const DWORD_OFFSET: usize, const HIGH: u32, const LOW: u32>;

impl<const DWORD_OFFSET: usize, const HIGH: u32, const LOW: u32> RegField<DWORD_OFFSET, HIGH, LOW> {
    const MASK: u32 = genmask_t!(u32, HIGH, LOW);
    const SHIFT: u32 = LOW;

    pub fn get_field(raw: &[u32]) -> u32 {
        (raw[DWORD_OFFSET] & Self::MASK) >> Self::SHIFT
    }

    pub fn set_field(raw: &mut [u32], value: u32) {
        raw[DWORD_OFFSET] =
            (!Self::MASK & raw[DWORD_OFFSET]) | (Self::MASK & (value << Self::SHIFT));
    }
}

/// Calculates 8-bit CRC over data.
///
/// This uses the CRC algorithm specified in the USB4 DROM specification.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc8 = 0xff;

    data.iter().for_each(|b| {
        crc8 ^= b;
        for _ in 0..=7 {
            if crc8 & 0x80 > 0 {
                crc8 <<= 1;
                crc8 ^= 0x07;
            } else {
                crc8 <<= 1;
            }
        }
    });

    crc8
}

/// Calculates 32-bit CRC over data.
///
/// This uses the CRC algorithm specified in the USB4 DROM specification.
pub fn crc32(data: &[u8]) -> u32 {
    const CRC32_TABLE: [u32; 256] = [
        0x00000000, 0xf26b8303, 0xe13b70f7, 0x1350f3f4, 0xc79a971f, 0x35f1141c, 0x26a1e7e8,
        0xd4ca64eb, 0x8ad958cf, 0x78b2dbcc, 0x6be22838, 0x9989ab3b, 0x4d43cfd0, 0xbf284cd3,
        0xac78bf27, 0x5e133c24, 0x105ec76f, 0xe235446c, 0xf165b798, 0x030e349b, 0xd7c45070,
        0x25afd373, 0x36ff2087, 0xc494a384, 0x9a879fa0, 0x68ec1ca3, 0x7bbcef57, 0x89d76c54,
        0x5d1d08bf, 0xaf768bbc, 0xbc267848, 0x4e4dfb4b, 0x20bd8ede, 0xd2d60ddd, 0xc186fe29,
        0x33ed7d2a, 0xe72719c1, 0x154c9ac2, 0x061c6936, 0xf477ea35, 0xaa64d611, 0x580f5512,
        0x4b5fa6e6, 0xb93425e5, 0x6dfe410e, 0x9f95c20d, 0x8cc531f9, 0x7eaeb2fa, 0x30e349b1,
        0xc288cab2, 0xd1d83946, 0x23b3ba45, 0xf779deae, 0x05125dad, 0x1642ae59, 0xe4292d5a,
        0xba3a117e, 0x4851927d, 0x5b016189, 0xa96ae28a, 0x7da08661, 0x8fcb0562, 0x9c9bf696,
        0x6ef07595, 0x417b1dbc, 0xb3109ebf, 0xa0406d4b, 0x522bee48, 0x86e18aa3, 0x748a09a0,
        0x67dafa54, 0x95b17957, 0xcba24573, 0x39c9c670, 0x2a993584, 0xd8f2b687, 0x0c38d26c,
        0xfe53516f, 0xed03a29b, 0x1f682198, 0x5125dad3, 0xa34e59d0, 0xb01eaa24, 0x42752927,
        0x96bf4dcc, 0x64d4cecf, 0x77843d3b, 0x85efbe38, 0xdbfc821c, 0x2997011f, 0x3ac7f2eb,
        0xc8ac71e8, 0x1c661503, 0xee0d9600, 0xfd5d65f4, 0x0f36e6f7, 0x61c69362, 0x93ad1061,
        0x80fde395, 0x72966096, 0xa65c047d, 0x5437877e, 0x4767748a, 0xb50cf789, 0xeb1fcbad,
        0x197448ae, 0x0a24bb5a, 0xf84f3859, 0x2c855cb2, 0xdeeedfb1, 0xcdbe2c45, 0x3fd5af46,
        0x7198540d, 0x83f3d70e, 0x90a324fa, 0x62c8a7f9, 0xb602c312, 0x44694011, 0x5739b3e5,
        0xa55230e6, 0xfb410cc2, 0x092a8fc1, 0x1a7a7c35, 0xe811ff36, 0x3cdb9bdd, 0xceb018de,
        0xdde0eb2a, 0x2f8b6829, 0x82f63b78, 0x709db87b, 0x63cd4b8f, 0x91a6c88c, 0x456cac67,
        0xb7072f64, 0xa457dc90, 0x563c5f93, 0x082f63b7, 0xfa44e0b4, 0xe9141340, 0x1b7f9043,
        0xcfb5f4a8, 0x3dde77ab, 0x2e8e845f, 0xdce5075c, 0x92a8fc17, 0x60c37f14, 0x73938ce0,
        0x81f80fe3, 0x55326b08, 0xa759e80b, 0xb4091bff, 0x466298fc, 0x1871a4d8, 0xea1a27db,
        0xf94ad42f, 0x0b21572c, 0xdfeb33c7, 0x2d80b0c4, 0x3ed04330, 0xccbbc033, 0xa24bb5a6,
        0x502036a5, 0x4370c551, 0xb11b4652, 0x65d122b9, 0x97baa1ba, 0x84ea524e, 0x7681d14d,
        0x2892ed69, 0xdaf96e6a, 0xc9a99d9e, 0x3bc21e9d, 0xef087a76, 0x1d63f975, 0x0e330a81,
        0xfc588982, 0xb21572c9, 0x407ef1ca, 0x532e023e, 0xa145813d, 0x758fe5d6, 0x87e466d5,
        0x94b49521, 0x66df1622, 0x38cc2a06, 0xcaa7a905, 0xd9f75af1, 0x2b9cd9f2, 0xff56bd19,
        0x0d3d3e1a, 0x1e6dcdee, 0xec064eed, 0xc38d26c4, 0x31e6a5c7, 0x22b65633, 0xd0ddd530,
        0x0417b1db, 0xf67c32d8, 0xe52cc12c, 0x1747422f, 0x49547e0b, 0xbb3ffd08, 0xa86f0efc,
        0x5a048dff, 0x8ecee914, 0x7ca56a17, 0x6ff599e3, 0x9d9e1ae0, 0xd3d3e1ab, 0x21b862a8,
        0x32e8915c, 0xc083125f, 0x144976b4, 0xe622f5b7, 0xf5720643, 0x07198540, 0x590ab964,
        0xab613a67, 0xb831c993, 0x4a5a4a90, 0x9e902e7b, 0x6cfbad78, 0x7fab5e8c, 0x8dc0dd8f,
        0xe330a81a, 0x115b2b19, 0x020bd8ed, 0xf0605bee, 0x24aa3f05, 0xd6c1bc06, 0xc5914ff2,
        0x37faccf1, 0x69e9f0d5, 0x9b8273d6, 0x88d28022, 0x7ab90321, 0xae7367ca, 0x5c18e4c9,
        0x4f48173d, 0xbd23943e, 0xf36e6f75, 0x0105ec76, 0x12551f82, 0xe03e9c81, 0x34f4f86a,
        0xc69f7b69, 0xd5cf889d, 0x27a40b9e, 0x79b737ba, 0x8bdcb4b9, 0x988c474d, 0x6ae7c44e,
        0xbe2da0a5, 0x4c4623a6, 0x5f16d052, 0xad7d5351,
    ];

    let mut crc32c: u32 = 0xffffffff;

    data.chunks(4).for_each(|chunk| {
        for byte in chunk {
            let i: usize = ((crc32c ^ *byte as u32) & 0xff) as usize;
            crc32c = (crc32c >> 8) ^ CRC32_TABLE[i];
        }
    });

    crc32c ^ 0xffffffff
}
