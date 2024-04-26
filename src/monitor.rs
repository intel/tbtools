// Thunderbolt/USB4 debug tools
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

//! Monitor events on the Thunderbolt/USB4 bus.

use nix::sys::{select, time};
use std::io::Result;
use std::os::fd::AsRawFd;
use std::time::Duration;

use crate::{Device, Kind};

/// Possible events the monitor can emit.
pub enum Event {
    /// Device has been added to the domain.
    Add(Device),
    /// Device has been removed from the domain.
    Remove(Device),
    /// Device has changes.
    Change(Device),
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
    ///                [None][`Option::None`] blocks forever.
    ///
    /// Returns `true` if there was an event, `false` otherwise.
    pub fn poll(&mut self, duration: Option<Duration>) -> Result<bool> {
        let mut readfds = select::FdSet::new();
        readfds.insert(self.socket.as_raw_fd());

        let mut tv: Option<time::TimeVal> = duration.map(|duration| {
            time::TimeVal::new(
                duration.as_secs().try_into().unwrap(),
                duration.subsec_micros().into(),
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

        let device = Device::parse(e.device())?;
        match e.event_type() {
            udev::EventType::Add => Some(Event::Add(device)),
            udev::EventType::Change => Some(Event::Change(device)),
            udev::EventType::Remove => Some(Event::Remove(device)),
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
