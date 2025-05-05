// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Monitor events on the Thunderbolt/USB4 bus.

use nix::sys::{select, time::TimeVal};
use std::{
    fmt::{self, Display},
    io::Result,
    os::fd::AsRawFd,
    time::Duration,
};

use crate::{Device, Kind};

/// Tunneling event in the domain.
#[derive(Debug, Clone)]
pub enum TunnelEvent {
    /// Tunnel was activated.
    Activated,
    /// Tunnel was changed somehow.
    Changed,
    /// Tunnel was torn down.
    Deactivated,
    /// Sub-optimal bandwidth available for the tunnel.
    LowBandwidth,
    /// Not enough bandwitdh available for the tunnel.
    NoBandwidth,
}

impl From<&str> for TunnelEvent {
    fn from(s: &str) -> Self {
        match s {
            "activated" => Self::Activated,
            "changed" => Self::Changed,
            "deactivated" => Self::Deactivated,
            "low bandwidth" => Self::LowBandwidth,
            "insufficient bandwidth" => Self::NoBandwidth,
            _ => panic!("unknown tunneling event"),
        }
    }
}

impl Display for TunnelEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::Activated => "activated",
            Self::Changed => "changed",
            Self::Deactivated => "deactivated",
            Self::LowBandwidth => "low bandwidth",
            Self::NoBandwidth => "insufficient bandwidth",
        };
        write!(f, "{}", s)
    }
}

/// Describes the type of the change event.
#[derive(Debug, Clone)]
pub enum ChangeEvent {
    /// Router was authorized or de-authorized.
    Router { authorized: u8 },
    /// Tunneling related change.
    Tunnel {
        /// Type of the tunnel event.
        event: TunnelEvent,
        /// Details of the tunneling event if available.
        details: Option<String>,
    },
}

/// Possible events the monitor can emit.
pub enum Event {
    /// Device has been added to the domain.
    Add(Device),
    /// Device has been removed from the domain.
    Remove(Device),
    /// Device has changes.
    Change(Device, Option<ChangeEvent>),
}

impl Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match *self {
            Self::Add(_) => "add",
            Self::Remove(_) => "remove",
            Self::Change(..) => "change",
        };
        write!(f, "{}", s)
    }
}

/// Monitors the bus for changes
pub struct Monitor {
    /// Internal reference to the udev monitor socket
    socket: udev::MonitorSocket,
}

impl Monitor {
    fn new(socket: udev::MonitorSocket) -> Self {
        Self { socket }
    }

    /// Poll for a new event.
    ///
    /// * `duration` - Timeout how long to wait until the function returns. Passing
    ///   [None][`Option::None`] blocks forever.
    ///
    /// Returns `true` if there was an event, `false` otherwise.
    pub fn poll(&mut self, duration: Option<Duration>) -> Result<bool> {
        let mut readfds = select::FdSet::new();
        readfds.insert(self.socket.as_raw_fd());

        let mut tv: Option<TimeVal> = duration.map(|duration| {
            TimeVal::new(
                duration.as_secs().try_into().unwrap(),
                #[allow(clippy::unnecessary_fallible_conversions)]
                duration.subsec_micros().try_into().unwrap(),
            )
        });
        let nfds = select::select(None, Some(&mut readfds), None, None, &mut tv)?;

        Ok(nfds > 0)
    }

    /// Returns iterator over the events currently available.
    pub fn iter(&self) -> &Self {
        self
    }

    /// Returns mutable iterator over the events currentl available.
    pub fn iter_mut(&mut self) -> &mut Self {
        self
    }
}

impl Iterator for Monitor {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        let e = self.socket.iter().next()?;

        match e.event_type() {
            udev::EventType::Add => Some(Event::Add(Device::parse(e.device())?)),
            udev::EventType::Remove => Some(Event::Remove(Device::parse(e.device())?)),
            udev::EventType::Change => {
                let udev = e.device();
                let kind = Kind::from(udev.devtype()?.to_str()?);

                let change_event = match kind {
                    Kind::Domain => {
                        if let Some(tunnel_event) =
                            udev.property_value("TUNNEL_EVENT").and_then(|e| e.to_str())
                        {
                            let details = udev
                                .property_value("TUNNEL_DETAILS")
                                .and_then(|d| d.to_str())
                                .map(|s| s.to_string());
                            Some(ChangeEvent::Tunnel {
                                event: TunnelEvent::from(tunnel_event),
                                details,
                            })
                        } else {
                            None
                        }
                    }
                    Kind::Router => {
                        if let Some(authorized) = udev
                            .property_value("AUTHORIZED")
                            .and_then(|a| a.to_str())
                            .map(|s| s.parse::<u8>().ok())
                        {
                            Some(ChangeEvent::Router {
                                authorized: authorized?,
                            })
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                Some(Event::Change(Device::parse(udev)?, change_event))
            }
            _ => None,
        }
    }
}

/// Builds a new `Monitor`.
///
/// # Examples
/// ```no_run
/// # use std::io;
/// use std::time::Duration;
/// use tbtools::{monitor, Kind};
///
/// # fn main() -> io::Result<()> {
/// // Build up a monitor that monitors routers on the bus.
/// let mut monitor = monitor::Builder::new()?.kind(Kind::Router)?.build()?;
///
/// // Poll for the changes.
/// if monitor.poll(Some(Duration::from_millis(500)))? {
///     for event in monitor.iter_mut() {
///         // Process the event.
///     }
/// }
/// # Ok(())
/// # }
/// ```
pub struct Builder {
    builder: udev::MonitorBuilder,
}

impl Builder {
    /// Creates a new Builder.
    ///
    /// By default all devices under Thunderbolt/USB4 bus are getting notifications. This can be
    /// tuned by calling [kind][`Builder::kind`].
    pub fn new() -> Result<Self> {
        let builder = udev::MonitorBuilder::new()?;
        Ok(Self { builder })
    }

    /// Add filter for specific device type only.
    ///
    /// * `kind` - specifies which kind of device is going to get notifications
    pub fn kind(mut self, kind: Kind) -> Result<Self> {
        self.builder = self
            .builder
            .match_subsystem_devtype("thunderbolt", kind.to_string())?;
        Ok(self)
    }

    /// Builds and returms the monitor.
    ///
    /// Consumes the `Builder`.
    pub fn build(self) -> Result<Monitor> {
        let socket = self.builder.listen()?;
        Ok(Monitor::new(socket))
    }
}
