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

use crate::debugfs::{self, Speed};
use crate::device::{find_device, Address};
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

fn read_results(path: &Path) -> Result<[u32; 3]> {
    read_dwords(path, MARGINING_RESULTS)
}

/// Which type of independent voltage margins are supported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndependentVoltage {
    /// Minimum between high and low margins is returned.
    Gen23Minimum,
    /// Either high or low margins is returned.
    Gen23Either,
    /// Both high and low margins are returned.
    Gen23Both,
    /// Minimum between high and low margins of the upper and lower eye is returned.
    Gen4Minimum,
    /// Both high and low margins are returned for upper and lower eye.
    Gen4Both,
}

/// Which type of independent timing margins are supported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IndependentTiming {
    /// Minimum between right and left margins is returned.
    Gen23Minimum,
    /// Either right or left margins is returned.
    Gen23Either,
    /// Both right and left margins are returned.
    Gen23Both,
    /// Minimum between right and left margins of the upper and lower eye is returned.
    Gen4Minimum,
    /// Both right and left margins are returned for upper and lower eye.
    Gen4Both,
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
    fn new(values: [u32; 3], speed: Speed) -> Self {
        let (hardware, software, has_time) = match speed {
            Speed::Gen4 => (
                usb4::margin::cap_2::ModesHW::get_bit(&values),
                usb4::margin::cap_2::ModesSW::get_bit(&values),
                usb4::margin::cap_2::Time::get_bit(&values),
            ),
            _ => (
                usb4::margin::cap_0::ModesHW::get_bit(&values),
                usb4::margin::cap_0::ModesSW::get_bit(&values),
                usb4::margin::cap_0::Time::get_bit(&values),
            ),
        };
        let all_lanes = usb4::margin::cap_0::MultiLane::get_bit(&values);

        let time = if has_time {
            let destructive = usb4::margin::cap_1::TimeDestr::get_bit(&values);

            let independent_margins = match speed {
                Speed::Gen4 => match usb4::margin::cap_2::TimeIndp::get_field(&values) {
                    usb4::margin::cap_2::TIME_INDP_MIN => IndependentTiming::Gen4Minimum,
                    usb4::margin::cap_2::TIME_INDP_BOTH => IndependentTiming::Gen4Both,
                    _ => panic!("Unsupported independent Gen4 timing margin caps value"),
                },
                _ => match usb4::margin::cap_1::TimeIndp::get_field(&values) {
                    usb4::margin::cap_1::TIME_INDP_MIN => IndependentTiming::Gen23Minimum,
                    usb4::margin::cap_1::TIME_INDP_EITHER => IndependentTiming::Gen23Either,
                    usb4::margin::cap_1::TIME_INDP_BOTH => IndependentTiming::Gen23Both,
                    _ => panic!("Unsupported independent timing margin caps value"),
                },
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

        let independent_voltage_margins = match speed {
            Speed::Gen4 => match usb4::margin::cap_2::VoltageIndp::get_field(&values) {
                usb4::margin::cap_0::VOLTAGE_INDP_MIN => IndependentVoltage::Gen4Minimum,
                usb4::margin::cap_0::VOLTAGE_INDP_BOTH => IndependentVoltage::Gen4Both,
                _ => panic!("Unsupported independent Gen4 voltage margin caps value"),
            },
            _ => match usb4::margin::cap_0::VoltageIndp::get_field(&values) {
                usb4::margin::cap_0::VOLTAGE_INDP_MIN => IndependentVoltage::Gen23Minimum,
                usb4::margin::cap_0::VOLTAGE_INDP_EITHER => IndependentVoltage::Gen23Either,
                usb4::margin::cap_0::VOLTAGE_INDP_BOTH => IndependentVoltage::Gen23Both,
                _ => panic!("Unsupported independent voltage margin caps value"),
            },
        };

        let (max_voltage_offset, voltage_steps) = match speed {
            Speed::Gen4 => (
                74.0 / 2.0 + usb4::margin::cap_2::MaxVoltageOffset::get_field(&values) as f64,
                usb4::margin::cap_2::VoltageSteps::get_field(&values),
            ),
            _ => (
                74.0 + usb4::margin::cap_0::MaxVoltageOffset::get_field(&values) as f64 * 2.0,
                usb4::margin::cap_0::VoltageSteps::get_field(&values),
            ),
        };

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

    fn with_path(path: &Path, speed: Speed) -> Result<Self> {
        Ok(Self::new(read_dwords(path, MARGINING_CAPS)?, speed))
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lanes {
    /// Run only on lane 0.
    Lane0,
    /// Run only on lane 1.
    Lane1,
    /// Run only on lane 2.
    Lane2,
    /// Run on all lanes simultaneusly.
    All,
}

impl Lanes {
    fn intersects_with(&self, other: Self) -> bool {
        *self == Lanes::All || other == Lanes::All || *self == other
    }
}

impl From<&str> for Lanes {
    fn from(s: &str) -> Self {
        match s {
            "0" => Self::Lane0,
            "1" => Self::Lane1,
            "2" => Self::Lane2,
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
            Self::Lane2 => "2",
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

/// Margining result value.
#[derive(Debug)]
pub enum ResultValue {
    /// Result exceeds the maximum.
    Exceeds(f64),
    /// Result is within the expected range.
    Ok(f64),
}

impl ResultValue {
    fn new(value: f64, exceeds: bool) -> Self {
        if exceeds {
            Self::Exceeds(value)
        } else {
            Self::Ok(value)
        }
    }
}

/// Holds either Gen 4 upper eye or lower eye result.
#[derive(Debug)]
pub enum LaneResultGen4Both {
    UpperEye(ResultValue),
    LowerEye(ResultValue),
}

impl LaneResultGen4Both {
    fn new(value: ResultValue, rhu: bool) -> Self {
        match rhu {
            true => Self::UpperEye(value),
            false => Self::LowerEye(value),
        }
    }
}

/// Holds voltage lane margining result.
#[derive(Debug)]
pub enum LaneVoltageResult {
    /// Result contains minimum between high and low voltage margins.
    Minimum(ResultValue),
    /// Results contains both high and low voltage margins.
    Both { low: ResultValue, high: ResultValue },
    /// Result contains high voltage margin.
    High(ResultValue),
    /// Result contains low voltage margin.
    Low(ResultValue),
    /// Result contains high and low voltage margins if both eyes.
    Gen4Both {
        low: LaneResultGen4Both,
        high: LaneResultGen4Both,
    },
}

/// Holds time lane margining result.
#[derive(Debug)]
pub enum LaneTimingResult {
    /// Result contains minimum between left and right time margins.
    Minimum(ResultValue),
    /// Result contains both left and right time margins.
    Both {
        left: ResultValue,
        right: ResultValue,
    },
    /// Result contains right time margin.
    Right(ResultValue),
    /// Result contains left time margin.
    Left(ResultValue),
    Gen4Both {
        left: LaneResultGen4Both,
        right: LaneResultGen4Both,
    },
}

/// Type of the margining result.
#[derive(Debug)]
pub enum LaneResult {
    /// Result is voltage margining.
    Voltage(LaneVoltageResult),
    /// Result is time margining.
    Timing(LaneTimingResult),
}

impl LaneResult {
    fn new(
        caps: &Caps,
        test: Test,
        rhu: bool,
        ll_result_value: ResultValue,
        hr_result_value: ResultValue,
    ) -> Self {
        match test {
            Test::Voltage => LaneResult::Voltage(match caps.independent_voltage_margins {
                IndependentVoltage::Gen23Minimum | IndependentVoltage::Gen4Minimum => {
                    LaneVoltageResult::Minimum(ll_result_value)
                }
                IndependentVoltage::Gen23Both => LaneVoltageResult::Both {
                    low: ll_result_value,
                    high: hr_result_value,
                },
                IndependentVoltage::Gen23Either => match rhu {
                    true => LaneVoltageResult::High(hr_result_value),
                    false => LaneVoltageResult::Low(hr_result_value),
                },
                IndependentVoltage::Gen4Both => LaneVoltageResult::Gen4Both {
                    high: LaneResultGen4Both::new(hr_result_value, rhu),
                    low: LaneResultGen4Both::new(ll_result_value, rhu),
                },
            }),
            Test::Time => LaneResult::Timing(match caps.time.unwrap().independent_margins {
                IndependentTiming::Gen23Minimum | IndependentTiming::Gen4Minimum => {
                    LaneTimingResult::Minimum(ll_result_value)
                }
                IndependentTiming::Gen23Both => LaneTimingResult::Both {
                    left: ll_result_value,
                    right: hr_result_value,
                },
                IndependentTiming::Gen23Either => match rhu {
                    true => LaneTimingResult::Right(hr_result_value),
                    false => LaneTimingResult::Left(hr_result_value),
                },
                IndependentTiming::Gen4Both => LaneTimingResult::Gen4Both {
                    right: LaneResultGen4Both::new(hr_result_value, rhu),
                    left: LaneResultGen4Both::new(ll_result_value, rhu),
                },
            }),
        }
    }
}

/// Results returned from a receiver lane margining operation.
///
/// These are returned from [`Margining::run()`] after successful execution.
#[derive(Debug)]
pub struct Results {
    result: [u32; 3],
    caps: Caps,
    test: Test,
    lanes: Lanes,
    rhu: bool,
    voltage_ratio: f64,
    time_ratio: f64,
}

impl Results {
    fn new(caps: &Caps, values: [u32; 3]) -> Self {
        let voltage_ratio = caps.max_voltage_offset / caps.voltage_steps as f64;

        let test = if usb4::margin::hw_res_0::Time::get_bit(&values) {
            Test::Time
        } else {
            Test::Voltage
        };

        let lanes = match usb4::margin::hw_res_0::LaneSelect::get_field(&values) {
            0 => Lanes::Lane0,
            1 => Lanes::Lane1,
            2 => Lanes::Lane2,
            7 => Lanes::All,
            lanes => panic!("Error: Unsupported lanes: {:#x}", lanes),
        };

        let time_ratio = if let Some(time) = caps.time {
            time.max_offset / time.steps as f64
        } else {
            0.0
        };

        let rhu = usb4::margin::hw_res_0::RHU::get_bit(&values);

        Self {
            result: values,
            caps: *caps,
            lanes,
            test,
            rhu,
            voltage_ratio,
            time_ratio,
        }
    }

    fn to_margin(&self, value: u32) -> f64 {
        match self.test {
            Test::Time => value as f64 * self.time_ratio,
            Test::Voltage => value as f64 * self.voltage_ratio,
        }
    }

    fn result_value(&self, lane: Lanes, rhu: bool) -> ResultValue {
        use usb4::margin::hw_res_1::*;
        use usb4::margin::hw_res_2::*;
        let (margin_value, exceeds) = match (lane, rhu) {
            (Lanes::Lane0, false) => (
                LowLeftMarginRX0::get_field(&self.result),
                LowLeftExceedsRX0::get_bit(&self.result),
            ),
            (Lanes::Lane0, true) => (
                HighRightMarginRX0::get_field(&self.result),
                HighRightExceedsRX0::get_bit(&self.result),
            ),
            (Lanes::Lane1, false) => (
                LowLeftMarginRX1::get_field(&self.result),
                LowLeftExceedsRX1::get_bit(&self.result),
            ),
            (Lanes::Lane1, true) => (
                HighRightMarginRX1::get_field(&self.result),
                HighRightExceedsRX1::get_bit(&self.result),
            ),
            (Lanes::Lane2, false) => (
                LowLeftMarginRX2::get_field(&self.result),
                LowLeftExceedsRX2::get_bit(&self.result),
            ),
            (Lanes::Lane2, true) => (
                HighRightMarginRX2::get_field(&self.result),
                HighRightExceedsRX2::get_bit(&self.result),
            ),
            (Lanes::All, _) => panic!("Invalid lanes"),
        };
        ResultValue::new(self.to_margin(margin_value), exceeds)
    }

    /// High (or right) margin values.
    ///
    /// Depending on which lane was selected returns tuple of values in either `mV` or `UI` for
    /// each margin.
    pub fn margins(&self) -> [Option<LaneResult>; 3] {
        let handle_lane = |l| {
            self.lanes.intersects_with(l).then(|| {
                LaneResult::new(
                    &self.caps,
                    self.test,
                    self.rhu,
                    self.result_value(l, true),
                    self.result_value(Lanes::Lane0, false),
                )
            })
        };
        [
            handle_lane(Lanes::Lane0),
            handle_lane(Lanes::Lane1),
            handle_lane(Lanes::Lane2),
        ]
    }

    /// Returns error counters used with software margining.
    pub fn error_counter(&self, lane: Lanes) -> (u32, u32) {
        let lane0_counter = usb4::margin::sw_err::RX0::get_field(&self.result);
        let lane1_counter = usb4::margin::sw_err::RX1::get_field(&self.result);
        let lane2_counter = usb4::margin::sw_err::RX2::get_field(&self.result);

        match lane {
            Lanes::Lane0 => (lane0_counter, 0u32),
            Lanes::Lane1 => (lane1_counter, 0u32),
            Lanes::Lane2 => (lane2_counter, 0u32),
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
        self.lanes
    }

    /// Sets lanes to be used for margining.
    pub fn set_lanes(&mut self, lanes: Lanes) {
        self.lanes = lanes
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
        Ok(Results::new(&self.caps, results))
    }

    /// Attaches margining to a given USB4 port or retimer.
    pub fn new(address: &Address) -> Result<Self> {
        let mut device = find_device(address)?.unwrap();
        let mut path_buf = debugfs::path_buf()?;
        let speed: Speed;

        match address {
            Address::Adapter {
                domain,
                route,
                adapter,
            } => {
                path_buf.push(format!("{}-{:x}", domain, route));
                path_buf.push(format!("port{}", adapter));

                device.read_adapters()?;

                if let Some(adapter) = device.adapter(*adapter) {
                    speed = adapter.link_speed();
                } else {
                    return Err(Error::from(ErrorKind::InvalidData));
                }
            }
            Address::Retimer {
                domain,
                route,
                adapter,
                index,
            } => {
                path_buf.push(format!("{}-{:x}:{}.{}", domain, route, adapter, index));

                let mut parent = find_device(&Address::Router {
                    domain: *domain,
                    route: *route,
                })?
                .unwrap();

                parent.read_adapters()?;

                if let Some(adapter) = parent.adapter(*adapter) {
                    speed = adapter.link_speed();
                } else {
                    return Err(Error::from(ErrorKind::InvalidData));
                }
            }
            _ => return Err(Error::from(ErrorKind::InvalidData)),
        }

        path_buf.push(MARGINING_DIR);

        let path = path_buf.as_path();
        let caps = match Caps::with_path(path, speed) {
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
        let margin = if (caps.independent_voltage_margins == IndependentVoltage::Gen23Either)
            || caps
                .time
                .is_some_and(|time| time.independent_margins == IndependentTiming::Gen23Either)
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
