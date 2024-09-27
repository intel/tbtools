// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Utilities to support receiver lane margining.
//!
//! The kernel must be compiled with `CONFIG_USB4_DEBUGFS_MARGINING=y`. for these to work.

use lazy_static::lazy_static;

use std::fmt;
use std::fs::{read_to_string, OpenOptions};
use std::io::{BufWriter, Error, ErrorKind, Result, Write};
use std::path::{Path, PathBuf};

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

fn read_attr(path: &Path, attr: &str) -> Result<String> {
    read_to_string(path.join(attr))
}

fn write_attr(path: &Path, attr: &str, value: &str) -> Result<()> {
    let path_buf = path.join(attr);

    let file = OpenOptions::new().write(true).open(path_buf)?;
    let mut buf = BufWriter::new(file);
    writeln!(&mut buf, "{value}")?;
    buf.flush()
}

fn read_ber_level_contour(path: &Path) -> Result<u32> {
    let value = read_attr(path, MARGINING_BER_LEVEL_CONTOUR)?;
    let caps = BER_LEVEL_RE.captures(&value).unwrap();

    Ok(caps[1].parse::<u32>().unwrap())
}

fn write_ber_level_contour(path: &Path, value: u32) -> Result<()> {
    write_attr(path, MARGINING_BER_LEVEL_CONTOUR, &value.to_string())
}

/// Reads `[value0] value1 value2` attribute and returns the one that is
/// currently selected (in brackets)
fn read_select(path: &Path, attr: &str) -> Result<String> {
    let value = read_attr(path, attr)?;
    let caps = SELECT_RE.captures(&value).unwrap();

    Ok(String::from(&caps[1]))
}

fn read_margin(path: &Path) -> Result<Margin> {
    let value: &str = &read_select(path, MARGINING_MARGIN)?;
    Ok(Margin::from(value))
}

fn write_margin(path: &Path, value: &Margin) -> Result<()> {
    write_attr(path, MARGINING_MARGIN, &value.to_string())
}

fn read_mode(path: &Path) -> Result<Mode> {
    let value: &str = &read_select(path, MARGINING_MODE)?;
    Ok(Mode::from(value))
}

fn write_mode(path: &Path, value: &Mode) -> Result<()> {
    write_attr(path, MARGINING_MODE, &value.to_string())
}

fn read_test(path: &Path) -> Result<Test> {
    let value: &str = &read_select(path, MARGINING_TEST)?;
    Ok(Test::from(value))
}

fn write_test(path: &Path, value: &Test) -> Result<()> {
    write_attr(path, MARGINING_TEST, &value.to_string())
}

fn read_lanes(path: &Path) -> Result<Lanes> {
    let value: &str = &read_select(path, MARGINING_LANES)?;
    Ok(Lanes::from(value))
}

fn write_lanes(path: &Path, value: &Lanes) -> Result<()> {
    write_attr(path, MARGINING_LANES, &value.to_string())
}

fn read_dwords<const NUM_DWORDS: usize>(path: &Path, attr: &str) -> Result<[u32; NUM_DWORDS]> {
    let value = read_attr(path, attr)?;
    let lines = value.split('\n');
    let mut dwords_iter = lines
        .filter(|line| line.starts_with("0x"))
        .map(|line| util::parse_hex::<u32>(line).unwrap())
        .chain(std::iter::repeat(0));
    Ok(std::array::from_fn(|_| dwords_iter.next().unwrap()))
}

fn read_results(path: &Path) -> Result<[u32; 2]> {
    read_dwords(path, MARGINING_RESULTS)
}

/// Which type of independent voltage margins are supported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndependentVoltage {
    /// Minimum between high and low margins is returned.
    Minimum,
    /// Either high or low margins is returned.
    Either,
    /// Both high and low margins are returned.
    Both,
}

/// Which type of independent timing margins are supported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndependentTiming {
    /// Minimum between right and left margins is returned.
    Minimum,
    /// Either right or left margins is returned.
    Either,
    /// Both right and left margins are returned.
    Both,
}

/// Timing margining specific capabilities result from `READ_LANE_MARGIN_CAP` USB4 port operation.
#[derive(Debug, Clone, Copy)]
pub struct TimeCaps {
    /// Is time margining supported.
    pub destructive: bool,
    /// Type of independent time margins.
    pub independent_margins: IndependentTiming,
    /// Maximum time offset in `UI` (Unit Interval).
    pub max_offset: f64,
    /// Number of time margining steps supported.
    pub steps: u32,
}

/// Margining capabilities result from `READ_LANE_MARGIN_CAP` USB4 port operation.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    /// Is hardware margining supported.
    pub hardware: bool,
    /// Is software margining supported.
    pub software: bool,
    /// Does margining run on individual lanes or all lanes at once.
    pub all_lanes: bool,
    /// Time margining capabilities if supported.
    pub time: Option<TimeCaps>,
    /// Type of independent voltage margins.
    pub independent_voltage_margins: IndependentVoltage,
    /// Maximum voltage offset in `mV`.
    pub max_voltage_offset: f64,
    /// Number of voltage margining steps supported.
    pub voltage_steps: u32,
}

impl Caps {
    fn new(values: [u32; 2]) -> Self {
        let hardware = usb4::margin::cap_0::ModesHW::get_bit(&values);
        let software = usb4::margin::cap_0::ModesSW::get_bit(&values);
        let all_lanes = usb4::margin::cap_0::MultiLane::get_bit(&values);

        let time = if usb4::margin::cap_0::Time::get_bit(&values) {
            let destructive = usb4::margin::cap_1::TimeDestr::get_bit(&values);
            let independent_margins = match usb4::margin::cap_1::TimeIndp::get_field(&values) {
                usb4::margin::cap_1::TIME_INDP_MIN => IndependentTiming::Minimum,
                usb4::margin::cap_1::TIME_INDP_EITHER => IndependentTiming::Either,
                usb4::margin::cap_1::TIME_INDP_BOTH => IndependentTiming::Both,
                _ => panic!("Unsupported independent timing margin caps value"),
            };
            let max_offset = {
                let value = usb4::margin::cap_1::TimeOffset::get_field(&values);
                0.2 + 0.01 * value as f64
            };
            let steps = usb4::margin::cap_1::TimeSteps::get_field(&values);
            Some(TimeCaps {
                destructive,
                independent_margins,
                max_offset,
                steps,
            })
        } else {
            None
        };

        let independent_voltage_margins = match usb4::margin::cap_0::VoltageIndp::get_field(&values)
        {
            usb4::margin::cap_0::VOLTAGE_INDP_MIN => IndependentVoltage::Minimum,
            usb4::margin::cap_0::VOLTAGE_INDP_EITHER => IndependentVoltage::Either,
            usb4::margin::cap_0::VOLTAGE_INDP_BOTH => IndependentVoltage::Both,
            _ => panic!("Unsupported independent voltage margin caps value"),
        };

        let max_voltage_offset = {
            let value = usb4::margin::cap_0::MaxVoltageOffset::get_field(&values);
            74.0 + value as f64 * 2.0
        };

        let voltage_steps = usb4::margin::cap_0::VoltageSteps::get_field(&values);

        Self {
            hardware,
            software,
            all_lanes,
            time,
            independent_voltage_margins,
            max_voltage_offset,
            voltage_steps,
        }
    }

    fn with_path(path: &Path) -> Result<Self> {
        Ok(Self::new(read_dwords(path, MARGINING_CAPS)?))
    }
}

/// Determines the margin in case independent margins are supported.
///
/// See also [`Caps::independent_voltage_margins`] and [`Caps::time.independent_margins`].
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
/// If [`Caps::all_lanes`] is `true` only `All` can be selected. Otherwise both lanes can be
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
/// Time margining can only be selected if [`Caps::time`] is `true`.
#[derive(Clone, Copy, Debug, PartialEq)]
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
    test: Test,
    voltage_ratio: f64,
    time_ratio: f64,
}

impl Results {
    fn new(caps: &Caps, test: Test, values: [u32; 2]) -> Self {
        let voltage_ratio = caps.max_voltage_offset / caps.voltage_steps as f64;

        let time_ratio = if let Some(time) = caps.time {
            time.max_offset / time.steps as f64
        } else {
            0.0
        };

        Self {
            result: values,
            test,
            voltage_ratio,
            time_ratio,
        }
    }

    /// Returns `true` if this result is from time margining.
    pub fn test(&self) -> Test {
        self.test
    }

    fn to_margin(&self, value: u32) -> f64 {
        match self.test {
            Test::Time => value as f64 * self.time_ratio,
            Test::Voltage => value as f64 * self.voltage_ratio,
        }
    }

    /// High (or right) margin values.
    ///
    /// Depending on which lane was selected returns tuple of values in either `mV` or `UI` for
    /// each margin.
    pub fn high_right_margin(&self, lane: &Lanes) -> (f64, f64) {
        let lane0_margin = self.to_margin(usb4::margin::hw_res_1::HighRightMarginRX0::get_field(
            &self.result,
        ));
        let lane1_margin = self.to_margin(usb4::margin::hw_res_1::HighRightMarginRX1::get_field(
            &self.result,
        ));

        match *lane {
            Lanes::Lane0 => (lane0_margin, 0.0),
            Lanes::Lane1 => (lane1_margin, 0.0),
            Lanes::All => (lane0_margin, lane1_margin),
        }
    }

    /// Returns `true` if high (or right) margin exceeds the maximum offset.
    pub fn high_right_margin_exceeds(&self, lane: &Lanes) -> (bool, bool) {
        let lane0_exceeds = usb4::margin::hw_res_1::HighRightExceedsRX0::get_bit(&self.result);
        let lane1_exceeds = usb4::margin::hw_res_1::HighRightExceedsRX1::get_bit(&self.result);

        match *lane {
            Lanes::Lane0 => (lane0_exceeds, false),
            Lanes::Lane1 => (lane1_exceeds, false),
            Lanes::All => (lane0_exceeds, lane1_exceeds),
        }
    }

    /// Returns low (or left) margin values in `mV` or `UI`.
    pub fn low_left_margin(&self, lane: &Lanes) -> (f64, f64) {
        let lane0_margin = self.to_margin(usb4::margin::hw_res_1::LowLeftMarginRX0::get_field(
            &self.result,
        ));
        let lane1_margin = self.to_margin(usb4::margin::hw_res_1::LowLeftMarginRX1::get_field(
            &self.result,
        ));

        match *lane {
            Lanes::Lane0 => (lane0_margin, 0.0),
            Lanes::Lane1 => (lane1_margin, 0.0),
            Lanes::All => (lane0_margin, lane1_margin),
        }
    }

    /// Returns `true` if low (or left) margin exceeds the maximum offset.
    pub fn low_left_margin_exceeds(&self, lane: &Lanes) -> (bool, bool) {
        let lane0_exceeds = usb4::margin::hw_res_1::LowLeftExceedsRX0::get_bit(&self.result);
        let lane1_exceeds = usb4::margin::hw_res_1::LowLeftExceedsRX1::get_bit(&self.result);
        match *lane {
            Lanes::Lane0 => (lane0_exceeds, false),
            Lanes::Lane1 => (lane1_exceeds, false),
            Lanes::All => (lane0_exceeds, lane1_exceeds),
        }
    }

    /// Returns error counters used with software margining.
    pub fn error_counter(&self, lane: &Lanes) -> (u32, u32) {
        let lane0_counter = usb4::margin::sw_err::RX0::get_field(&self.result);
        let lane1_counter = usb4::margin::sw_err::RX1::get_field(&self.result);

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
/// let mut margining = Margining::new(&address)?;
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
    path: PathBuf,
}

impl Margining {
    /// Returns the capabilities of the USB4 port or retimer.
    pub fn caps(&self) -> Caps {
        self.caps
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
        self.test
    }

    /// Returns `true` if time margining is currently selected.
    pub fn is_time(&self) -> bool {
        self.test == Test::Time
    }

    /// Sets desired margining "test".
    pub fn set_test(&mut self, test: &Test) {
        self.test = *test;
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
        Ok(Results::new(&self.caps, self.test, results))
    }

    /// Attaches margining to a given USB4 port or retimer.
    pub fn new(address: &Address) -> Result<Self> {
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

        let path = path_buf.as_path();
        let caps = match Caps::with_path(path) {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                eprintln!("{}", MARGINING_HELP);
                Err(err)
            }
            Err(err) => Err(err),
            Ok(caps) => Ok(caps),
        }?;

        // Check that the margining is actually supported. Some routers such as the Anker one does
        // not support margining even though it's spec violation.

        if !caps.hardware && !caps.software {
            return Err(Error::from(ErrorKind::Unsupported));
        }

        let ber_level_contour = if caps.hardware {
            Some(read_ber_level_contour(path)?)
        } else {
            None
        };

        let margin = if caps.independent_voltage_margins == IndependentVoltage::Either
            || caps
                .time
                .is_some_and(|time| time.independent_margins == IndependentTiming::Either)
        {
            Some(read_margin(path)?)
        } else {
            None
        };

        let test = read_test(path)?;
        let mode = read_mode(path)?;
        let lanes = read_lanes(path)?;

        Ok(Margining {
            caps,
            margin,
            mode,
            ber_level_contour,
            lanes,
            test,
            path: path_buf,
        })
    }
}
