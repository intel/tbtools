// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Utilities to support receiver lane margining.
//!
//! The kernel must be compiled with `CONFIG_USB4_DEBUGFS_MARGINING=y`. for these to work.

use lazy_static::lazy_static;

use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Error, ErrorKind, Read, Result, Write};
use std::path::PathBuf;

use regex::Regex;

use crate::debugfs;
use crate::device::Address;
use crate::usb4;
use crate::util;

lazy_static! {
    static ref BER_LEVEL_RE: Regex = Regex::new(r".+ \((\d+)\)").unwrap();
    static ref SELECT_RE: Regex = Regex::new(r"\[(\w+)\]").unwrap();
}

// Margining directory and the possible attributes.
const MARGINING_DIR: &str = "margining";
const MARGINING_BER_LEVEL_CONTOUR: &str = "ber_level_contour";
const MARGINING_CAPS: &str = "caps";
const MARGINING_LANES: &str = "lanes";
const MARGINING_MARGIN: &str = "margin";
const MARGINING_MODE: &str = "mode";
const MARGINING_RESULTS: &str = "results";
const MARGINING_RUN: &str = "run";
const MARGINING_TEST: &str = "test";

const MARGINING_HELP: &str = "Note margining support needs to be built into the Thunderbolt driver
by setting following in your kernel .config:

    CONFIG_USB4_DEBUGFS_WRITE=y
    CONFIG_USB4_DEBUGFS_MARGINING=y
";

fn read_attr(path: &str, attr: &str) -> Result<String> {
    let mut path_buf = PathBuf::new();
    path_buf.push(path);
    path_buf.push(attr);

    let mut file = File::open(path_buf)?;
    let mut value = String::new();
    file.read_to_string(&mut value)?;

    Ok(value)
}

fn write_attr(path: &str, attr: &str, value: &str) -> Result<()> {
    let mut path_buf = PathBuf::new();
    path_buf.push(path);
    path_buf.push(attr);

    let file = OpenOptions::new().write(true).open(path_buf)?;
    let mut buf = BufWriter::new(file);
    writeln!(&mut buf, "{value}")?;
    buf.flush()
}

fn read_ber_level_contour(path: &str) -> Result<u32> {
    let value = read_attr(path, MARGINING_BER_LEVEL_CONTOUR)?;
    let caps = BER_LEVEL_RE.captures(&value).unwrap();

    Ok(caps[1].parse::<u32>().unwrap())
}

fn write_ber_level_contour(path: &str, value: u32) -> Result<()> {
    write_attr(path, MARGINING_BER_LEVEL_CONTOUR, &value.to_string())
}

/// Reads `[value0] value1 value2` attribute and returns the one that is
/// currently selected (in brackets)
fn read_select(path: &str, attr: &str) -> Result<String> {
    let value = read_attr(path, attr)?;
    let caps = SELECT_RE.captures(&value).unwrap();

    Ok(String::from(&caps[1]))
}

fn read_margin(path: &str) -> Result<Margin> {
    let value: &str = &read_select(path, MARGINING_MARGIN)?;
    Ok(Margin::from(value))
}

fn write_margin(path: &str, value: &Margin) -> Result<()> {
    write_attr(path, MARGINING_MARGIN, &value.to_string())
}

fn read_mode(path: &str) -> Result<Mode> {
    let value: &str = &read_select(path, MARGINING_MODE)?;
    Ok(Mode::from(value))
}

fn write_mode(path: &str, value: &Mode) -> Result<()> {
    write_attr(path, MARGINING_MODE, &value.to_string())
}

fn read_test(path: &str) -> Result<Test> {
    let value: &str = &read_select(path, MARGINING_TEST)?;
    Ok(Test::from(value))
}

fn write_test(path: &str, value: &Test) -> Result<()> {
    write_attr(path, MARGINING_TEST, &value.to_string())
}

fn read_lanes(path: &str) -> Result<Lanes> {
    let value: &str = &read_select(path, MARGINING_LANES)?;
    Ok(Lanes::from(value))
}

fn write_lanes(path: &str, value: &Lanes) -> Result<()> {
    write_attr(path, MARGINING_LANES, &value.to_string())
}

fn read_double_dwords(path: &str, attr: &str) -> Result<(u32, u32)> {
    let value = read_attr(path, attr)?;
    let lines = value.split('\n');
    let dwords: Vec<_> = lines
        .filter(|line| line.starts_with("0x"))
        .map(|line| util::parse_hex::<u32>(line).unwrap())
        .collect();
    Ok((dwords[0], *(dwords.get(1).unwrap_or(&0))))
}

fn read_caps(path: &str) -> Result<Caps> {
    Ok(Caps::from(read_double_dwords(path, MARGINING_CAPS)?))
}

fn read_results(path: &str) -> Result<(u32, u32)> {
    read_double_dwords(path, MARGINING_RESULTS)
}

/// Margining capabilities result from `READ_LANE_MARGIN_CAP` USB4 port operation.
#[derive(Debug)]
pub struct Caps(u32, u32);

impl Caps {
    /// Is hardware margining supported.
    pub fn hardware(&self) -> bool {
        (self.0 & usb4::MARGIN_CAP_0_MODES_HW) == usb4::MARGIN_CAP_0_MODES_HW
    }

    /// Is software marginint supported.
    pub fn software(&self) -> bool {
        (self.0 & usb4::MARGIN_CAP_0_MODES_SW) == usb4::MARGIN_CAP_0_MODES_SW
    }

    /// Does the margining run on individual lanes or all lanes at once.
    pub fn all_lanes(&self) -> bool {
        (self.0 & usb4::MARGIN_CAP_0_MULTI_LANE) == usb4::MARGIN_CAP_0_MULTI_LANE
    }

    /// Is time margining supported.
    pub fn time(&self) -> bool {
        (self.0 & usb4::MARGIN_CAP_0_TIME) == usb4::MARGIN_CAP_0_TIME
    }

    /// Is time margining destructive.
    pub fn time_is_destructive(&self) -> bool {
        if self.time() {
            return (self.1 & usb4::MARGIN_CAP_1_TIME_DESTR) == usb4::MARGIN_CAP_1_TIME_DESTR;
        }
        false
    }

    /// Independent voltage margins supported.
    pub fn independent_voltage_margins(&self) -> bool {
        (self.0 & usb4::MARGIN_CAP_0_VOLTAGE_INDP_MASK) >> usb4::MARGIN_CAP_0_VOLTAGE_INDP_SHIFT > 0
    }

    /// Independent time margins supported (only if [`time()`](`Self::time()`) returns `true`).
    pub fn independent_time_margins(&self) -> bool {
        if self.time() {
            return (self.1 & usb4::MARGIN_CAP_1_TIME_INDP_MASK)
                >> usb4::MARGIN_CAP_1_TIME_INDP_SHIFT
                > 0;
        }
        false
    }

    /// Maximum voltage offset in `mV`.
    pub fn max_voltage_offset(&self) -> f64 {
        let value = (self.0 & usb4::MARGIN_CAP_0_MAX_VOLTAGE_OFFSET_MASK)
            >> usb4::MARGIN_CAP_0_MAX_VOLTAGE_OFFSET_SHIFT;
        74.0 + value as f64 * 2.0
    }

    /// Number of voltage margining steps supported.
    pub fn voltage_steps(&self) -> u32 {
        (self.0 & usb4::MARGIN_CAP_0_VOLTAGE_STEPS_MASK) >> usb4::MARGIN_CAP_0_VOLTAGE_STEPS_SHIFT
    }

    /// Maximum time margining offset in `UI` (Unit Interval).
    pub fn max_time_offset(&self) -> f64 {
        let value =
            (self.1 & usb4::MARGIN_CAP_1_TIME_OFFSET_MASK) >> usb4::MARGIN_CAP_1_TIME_OFFSET_SHIFT;
        0.2 + 0.01 * value as f64
    }

    /// Number of time margining steps supported.
    pub fn time_steps(&self) -> u32 {
        (self.1 & usb4::MARGIN_CAP_1_TIME_STEPS_MASK) >> usb4::MARGIN_CAP_1_TIME_STEPS_SHIFT
    }

    fn results(&self, time: bool, values: (u32, u32)) -> Results {
        let voltage_ratio = self.max_voltage_offset() / self.voltage_steps() as f64;

        let time_ratio = if self.time() {
            self.max_time_offset() / self.time_steps() as f64
        } else {
            0.0
        };

        Results {
            result: [values.0, values.1],
            time,
            voltage_ratio,
            time_ratio,
        }
    }

    fn from(values: (u32, u32)) -> Self {
        Caps(values.0, values.1)
    }
}

/// Determines the margin in case independent margins are supported.
///
/// See also [`Caps::independent_voltage_margins()`] and [`Caps::independent_time_margins()`].
#[derive(Clone, Debug)]
pub enum Margin {
    /// Low voltage margin.
    Low,
    /// High voltage margin.
    High,
    /// Left time margin.
    Left,
    /// Right time margin.
    Right,
}

impl From<&str> for Margin {
    fn from(s: &str) -> Self {
        match s {
            "low" => Self::Low,
            "high" => Self::High,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => panic!("Error: unsupported margin"),
        }
    }
}

impl fmt::Display for Margin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val = match *self {
            Self::Low => "low",
            Self::High => "high",
            Self::Left => "left",
            Self::Right => "right",
        };
        write!(f, "{}", val)
    }
}

/// Determines margining mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// Hardware margining.
    Hardware,
    /// Software margining.
    Software,
}

impl From<&str> for Mode {
    fn from(s: &str) -> Self {
        match s {
            "hardware" => Self::Hardware,
            "software" => Self::Software,
            _ => panic!("Error: unsupported margining mode"),
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val = match *self {
            Self::Hardware => "hardware",
            Self::Software => "software",
        };
        write!(f, "{}", val)
    }
}

/// Selected lanes used for margining.
///
/// If [`Caps::all_lanes()`] returns `true` only `All` can be selected. Otherwise both lanes can be
/// used separately.
#[derive(Clone, Debug, PartialEq)]
pub enum Lanes {
    /// Run only on lane 0.
    Lane0,
    /// Run only on lane 1.
    Lane1,
    /// Run on all lanes simultaneusly.
    All,
}

impl From<&str> for Lanes {
    fn from(s: &str) -> Self {
        match s {
            "0" => Self::Lane0,
            "1" => Self::Lane1,
            "all" => Self::All,
            _ => panic!("Error: unsupported lanes configuration"),
        }
    }
}

impl fmt::Display for Lanes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val = match *self {
            Self::Lane0 => "0",
            Self::Lane1 => "1",
            Self::All => "all",
        };
        write!(f, "{}", val)
    }
}

/// Selected margining test.
///
/// Time margining can only be selected if [`Caps::time()`] returns `true`.
#[derive(Clone, Debug, PartialEq)]
pub enum Test {
    /// Run voltage margining.
    Voltage,
    /// Run time margining.
    Time,
}

impl From<&str> for Test {
    fn from(s: &str) -> Self {
        match s {
            "voltage" => Self::Voltage,
            "time" => Self::Time,
            _ => panic!("Error: unsupported margining test"),
        }
    }
}

impl fmt::Display for Test {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let val = match *self {
            Self::Voltage => "voltage",
            Self::Time => "time",
        };
        write!(f, "{}", val)
    }
}

/// Results returned from a receiver lane margining operation.
///
/// These are returned from [`Margining::run()`] after successful execution.
#[derive(Debug)]
pub struct Results {
    result: [u32; 2],
    time: bool,
    voltage_ratio: f64,
    time_ratio: f64,
}

impl Results {
    /// Returns `true` if this result is from time margining.
    pub fn time(&self) -> bool {
        self.time
    }

    fn to_margin(&self, value: u32) -> f64 {
        if self.time() {
            value as f64 * self.time_ratio
        } else {
            value as f64 * self.voltage_ratio
        }
    }

    /// High (or right) margin values.
    ///
    /// Depending on which lane was selected returns tuple of values in either `mV` or `UI` for
    /// each margin.
    pub fn high_right_margin(&self, lane: &Lanes) -> (f64, f64) {
        let lane0_margin = self.to_margin(self.result[1] & usb4::MARGIN_HW_RES_1_MARGIN_MASK);
        let lane1_margin = self.to_margin(
            (self.result[1] >> usb4::MARGIN_HW_RES_1_RX1_RH_MARGIN_SHIFT)
                & usb4::MARGIN_HW_RES_1_MARGIN_MASK,
        );

        match *lane {
            Lanes::Lane0 => (lane0_margin, 0.0),
            Lanes::Lane1 => (lane1_margin, 0.0),
            Lanes::All => (lane0_margin, lane1_margin),
        }
    }

    /// Returns `true` if high (or right) margin exceeds the maximum offset.
    pub fn high_right_margin_exceeds(&self, lane: &Lanes) -> (bool, bool) {
        let lane0_exceeds =
            (self.result[1] & usb4::MARGIN_HW_RES_1_EXCEEDS) == usb4::MARGIN_HW_RES_1_EXCEEDS;
        let lane1_exceeds = ((self.result[1] >> usb4::MARGIN_HW_RES_1_RX1_RH_MARGIN_SHIFT)
            & usb4::MARGIN_HW_RES_1_EXCEEDS)
            == usb4::MARGIN_HW_RES_1_EXCEEDS;

        match *lane {
            Lanes::Lane0 => (lane0_exceeds, false),
            Lanes::Lane1 => (lane1_exceeds, false),
            Lanes::All => (lane0_exceeds, lane1_exceeds),
        }
    }

    /// Returns low (or left) margin values in `mV` or `UI`.
    pub fn low_left_margin(&self, lane: &Lanes) -> (f64, f64) {
        let lane0_margin = self.to_margin(
            (self.result[1] >> usb4::MARGIN_HW_RES_1_RX0_LL_MARGIN_SHIFT)
                & usb4::MARGIN_HW_RES_1_MARGIN_MASK,
        );
        let lane1_margin = self.to_margin(
            (self.result[1] >> usb4::MARGIN_HW_RES_1_RX1_LL_MARGIN_SHIFT)
                & usb4::MARGIN_HW_RES_1_MARGIN_MASK,
        );
        match *lane {
            Lanes::Lane0 => (lane0_margin, 0.0),
            Lanes::Lane1 => (lane1_margin, 0.0),
            Lanes::All => (lane0_margin, lane1_margin),
        }
    }

    /// Returns `true` if low (or left) margin exceeds the maximum offset.
    pub fn low_left_margin_exceeds(&self, lane: &Lanes) -> (bool, bool) {
        let lane0_exceeds = ((self.result[1] >> usb4::MARGIN_HW_RES_1_RX0_LL_MARGIN_SHIFT)
            & usb4::MARGIN_HW_RES_1_EXCEEDS)
            == usb4::MARGIN_HW_RES_1_EXCEEDS;
        let lane1_exceeds = ((self.result[1] >> usb4::MARGIN_HW_RES_1_RX1_LL_MARGIN_SHIFT)
            & usb4::MARGIN_HW_RES_1_EXCEEDS)
            == usb4::MARGIN_HW_RES_1_EXCEEDS;
        match *lane {
            Lanes::Lane0 => (lane0_exceeds, false),
            Lanes::Lane1 => (lane1_exceeds, false),
            Lanes::All => (lane0_exceeds, lane1_exceeds),
        }
    }

    /// Returns error counters used with software margining.
    pub fn error_counter(&self, lane: &Lanes) -> (u32, u32) {
        let lane0_counter = self.result[0] & usb4::MARGIN_SW_ERR_RX0_MASK;
        let lane1_counter =
            (self.result[0] & usb4::MARGIN_SW_ERR_RX1_MASK) >> usb4::MARGIN_SW_ERR_RX1_SHIFT;

        match *lane {
            Lanes::Lane0 => (lane0_counter, 0u32),
            Lanes::Lane1 => (lane1_counter, 0u32),
            Lanes::All => (lane0_counter, lane1_counter),
        }
    }
}

/// Main interface to margining.
///
/// Each entity (USB4 port, retimer) that is capable of running receiver lane margining can be
/// presented by this object.
///
/// # Examples
/// ```no_run
/// # use std::io;
/// use tbtools::Address;
/// use tbtools::margining::Margining;
///
/// # fn main() -> io::Result<()> {
/// // Run margining on host router, first USB4 port.
/// let address = Address::Adapter { domain: 0, route: 0, adapter: 1 };
/// let mut margining = Margining::from(&address)?;
///
/// // Do additional configuration according to margining.caps().
/// // ...
///
/// let results = margining.run()?;
/// // Parse results.
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Margining {
    caps: Caps,
    margin: Option<Margin>,
    mode: Mode,
    ber_level_contour: Option<u32>,
    lanes: Lanes,
    test: Test,
    path: String,
}

impl Margining {
    /// Returns the capabilities of the USB4 port or retimer.
    pub fn caps(&self) -> Caps {
        Caps(self.caps.0, self.caps.1)
    }

    /// Returns current BER level contour value if supported.
    pub fn ber_level_contour(&self) -> Option<u32> {
        self.ber_level_contour
    }

    /// Sets new BER level contour value.
    pub fn set_ber_level_contour(&mut self, ber_level_contour: u32) {
        self.ber_level_contour = Some(ber_level_contour)
    }

    /// Returns the lanes currently used for margining.
    pub fn lanes(&self) -> Lanes {
        self.lanes.clone()
    }

    /// Sets lanes to be used for margining.
    pub fn set_lanes(&mut self, lanes: &Lanes) {
        self.lanes = lanes.clone()
    }

    /// Returns currently selected margin.
    pub fn margin(&self) -> Option<Margin> {
        if let Some(margin) = &self.margin {
            return Some(margin.clone());
        }
        None
    }

    /// Selects margin.
    pub fn set_margin(&mut self, margin: &Margin) {
        self.margin = Some(margin.clone())
    }

    /// Returns current margining mode.
    pub fn mode(&self) -> Mode {
        self.mode.clone()
    }

    /// Sets current margining mode.
    pub fn set_mode(&mut self, mode: &Mode) {
        self.mode = mode.clone()
    }

    /// Returns `true` if current mode is hardware margining.
    pub fn is_hardware(&self) -> bool {
        self.mode == Mode::Hardware
    }

    /// Returns `true` if current mode is software margining.
    pub fn is_software(&self) -> bool {
        !self.is_hardware()
    }

    /// Returns which "test" is selected.
    pub fn test(&self) -> Test {
        self.test.clone()
    }

    /// Returns `true` if time margining is currently selected.
    pub fn is_time(&self) -> bool {
        self.test == Test::Time
    }

    /// Sets desired margining "test".
    pub fn set_test(&mut self, test: &Test) {
        self.test = test.clone()
    }

    /// Runs margining according to the configured settings.
    ///
    /// Returns [`Results`] object if the test succeeded. If there was an error a [`Result`] is
    /// returned instead. This function can be called several times, changing parameters if needed.
    pub fn run(&mut self) -> Result<Results> {
        if self.mode == Mode::Hardware {
            write_ber_level_contour(&self.path, self.ber_level_contour.unwrap())?;
        }

        write_lanes(&self.path, &self.lanes)?;
        write_mode(&self.path, &self.mode)?;
        write_test(&self.path, &self.test)?;

        if let Some(margin) = &self.margin {
            write_margin(&self.path, margin)?;
        }

        // Start the test
        write_attr(&self.path, MARGINING_RUN, "1")?;

        // Read back results
        let results = read_results(&self.path)?;
        Ok(self.caps.results(self.is_time(), results))
    }

    /// Attaches margining to a given USB4 port or retimer.
    pub fn from(address: &Address) -> Result<Self> {
        let mut path_buf = debugfs::path_buf()?;

        match address {
            Address::Adapter {
                domain,
                route,
                adapter,
            } => {
                path_buf.push(format!("{}-{:x}", domain, route));
                path_buf.push(format!("port{}", adapter));
            }
            Address::Retimer {
                domain,
                route,
                adapter,
                index,
            } => {
                path_buf.push(format!("{}-{:x}:{}.{}", domain, route, adapter, index));
            }
            _ => return Err(Error::from(ErrorKind::InvalidData)),
        }

        path_buf.push(MARGINING_DIR);

        let path = String::from(path_buf.to_str().unwrap());
        let caps = match read_caps(&path) {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                eprintln!("{}", MARGINING_HELP);
                Err(err)
            }
            Err(err) => Err(err),
            Ok(caps) => Ok(caps),
        }?;

        // Check that the margining is actually supported. Some routers such as the Anker one does
        // not support margining even though it's spec violation.

        if !caps.hardware() && !caps.software() {
            return Err(Error::from(ErrorKind::Unsupported));
        }

        let ber_level_contour = if caps.hardware() {
            Some(read_ber_level_contour(&path)?)
        } else {
            None
        };
        let margin = if caps.independent_voltage_margins() || caps.independent_time_margins() {
            Some(read_margin(&path)?)
        } else {
            None
        };

        let test = read_test(&path)?;
        let mode = read_mode(&path)?;
        let lanes = read_lanes(&path)?;

        Ok(Margining {
            caps,
            margin,
            mode,
            ber_level_contour,
            lanes,
            test,
            path,
        })
    }
}
