// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//!Miscellaneous utility functions.

use std::{
    fs,
    io::{self, Error, ErrorKind},
    str::FromStr,
};

use lazy_static::lazy_static;
use nix::sys::time::TimeVal;
use num_traits::Num;
use regex::Regex;
use uuid;

/// Similar to kernel's `GENMASK()` macro.
///
/// # Examples
/// ```
/// use tbtools::genmask;
///
/// const TMU_RTR_CS_0_FREQ_WINDOW_MASK: u32 = genmask!(26, 16);
/// const TMU_RTR_CS_3_TS_PACKET_INTERVAL_MASK: u32 = genmask!(31, 16);
/// ```
#[macro_export]
macro_rules! genmask {
    ($high:expr, $low:expr) => {{
        u32::MAX - (1u32 << $low) + 1u32 & (u32::MAX >> (u32::BITS - 1 - $high))
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
        if b.is_ascii_alphanumeric() {
            s.push_str(&format!("{}", *b as char));
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
            if let Some(seconds) = parse_number::<i64>(&caps[1]) {
                return Ok(TimeVal::new(seconds, 0));
            }
        }
    }

    Err(Error::from(ErrorKind::Unsupported))
}
