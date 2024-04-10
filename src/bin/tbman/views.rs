// Thunderbolt/USB4 live device manager
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use crate::theme;
use cursive::{theme::Style, utils::span::SpannedString, view::View, Printer, Vec2, XY};
use tbtools::debugfs::{Adapter, BitFields, Path, State, Type};

/// View to show device adapters.
pub struct AdapterView {
    adapters: Vec<SpannedString<Style>>,
    changed: bool,
}

impl AdapterView {
    /// Creates a new AdapterView with no adapters.
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
            changed: true,
        }
    }

    fn protocol_state(&self, adapter: &Adapter, line: &mut SpannedString<Style>) {
        match adapter.kind() {
            Type::PcieDown | Type::PcieUp => {
                if let Some(reg) = adapter.register_by_name("ADP_PCIE_CS_0") {
                    if let Some(field) = reg.field_by_name("LTSSM") {
                        let v = reg.field_value(field);
                        match field.value_name(v) {
                            Some("L0 state") => {
                                line.append_styled(
                                    format!("{:^18}", "L0"),
                                    theme::adapter_active(),
                                );
                            }
                            Some("L1 state") => {
                                line.append_styled(format!("{:^18}", "L1"), theme::adapter_pm());
                            }
                            Some("L2 state") => {
                                line.append_styled(format!("{:^18}", "L2"), theme::adapter_pm());
                            }
                            Some("Disabled state") => {
                                line.append_styled(
                                    format!("{:^18}", "Disabled"),
                                    theme::adapter_disabled(),
                                );
                            }
                            Some("Hot Reset state") => {
                                line.append_styled(
                                    format!("{:^18}", "Hot Reset"),
                                    theme::adapter_disabled(),
                                );
                            }
                            Some(state) => {
                                line.append_styled(
                                    format!("{:^18}", state.trim_end_matches(" state")),
                                    theme::adapter_training(),
                                );
                            }
                            None => (),
                        }
                    }
                }
            }

            Type::Usb3Down | Type::Usb3Up => {
                if let Some(reg) = adapter.register_by_name("ADP_USB3_GX_CS_4") {
                    if let Some(field) = reg.field_by_name("PLS") {
                        let v = reg.field_value(field);
                        match field.value_name(v) {
                            Some("U0 state") => {
                                line.append_styled(
                                    format!("{:^18}", "U0"),
                                    theme::adapter_active(),
                                );
                            }
                            Some("U2 state") => {
                                line.append_styled(format!("{:^18}", "U2"), theme::adapter_pm());
                            }
                            Some("U3 state") => {
                                line.append_styled(format!("{:^18}", "U3"), theme::adapter_pm());
                            }
                            Some("Disabled state") => {
                                line.append_styled(
                                    format!("{:^15}", "Disabled"),
                                    theme::adapter_disabled(),
                                );
                            }
                            Some("Hot Reset state") => {
                                line.append_styled(
                                    format!("{:^18}", "Hot Reset"),
                                    theme::adapter_disabled(),
                                );
                            }
                            Some(state) => {
                                line.append_styled(
                                    format!("{:^18}", state.trim_end_matches(" state")),
                                    theme::adapter_training(),
                                );
                            }
                            None => (),
                        }
                    }
                }
            }

            _ => line.append_styled(format!("{:^18}", "Enabled"), theme::adapter_enabled()),
        }
    }

    fn format_adapter(&self, adapter: &Adapter) -> SpannedString<Style> {
        let mut line = SpannedString::new();

        line.append_styled(format!("{:>2}: ", adapter.adapter()), theme::dialog_label());

        if adapter.is_lane() || adapter.is_protocol() {
            let mut kind = if adapter.is_lane0() {
                String::from("Lane 0")
            } else if adapter.is_lane1() {
                String::from("Lane 1")
            } else {
                adapter.kind().to_string()
            };

            if adapter.is_upstream() {
                kind.push_str(" (upstream)");
            }

            line.append(format!("{:<28}", kind));

            match adapter.state() {
                State::Disabled => {
                    line.append_styled(format!("{:^18}", "Disabled"), theme::adapter_disabled());
                }
                State::Enabled => {
                    self.protocol_state(adapter, &mut line);
                }
                State::Training => line.append_styled(
                    format!("{:^18}", "Training/Bonding"),
                    theme::adapter_training(),
                ),
                State::Cl0 => {
                    line.append_styled(format!("{:^18}", "CL0"), theme::adapter_active());
                }
                State::Cl0sTx => {
                    line.append_styled(format!("{:^18}", "CL0s Tx"), theme::adapter_pm());
                }
                State::Cl0sRx => {
                    line.append_styled(format!("{:^18}", "CL0s Rx"), theme::adapter_pm());
                }
                State::Cl1 => {
                    line.append_styled(format!("{:^18}", "CL1"), theme::adapter_pm());
                }
                State::Cl2 => {
                    line.append_styled(format!("{:^18}", "CL2"), theme::adapter_pm());
                }
                State::Cld => {
                    line.append_styled(format!("{:^18}", "CLd"), theme::adapter_inactive());
                }
                _ => {
                    line.append_styled(format!("{:^18}", "Unknown"), theme::adapter_disabled());
                }
            }
        } else {
            line.append_styled("Not implemented", theme::adapter_not_implemented());
        }

        line
    }

    pub fn clear(&mut self) {
        self.adapters.clear();
        self.changed = true;
    }

    /// Add adapters to the view.
    ///
    /// This will force the view to refresh.
    pub fn add_adapters(&mut self, adapters: &Vec<Adapter>) {
        for adapter in adapters {
            self.adapters.push(self.format_adapter(adapter));
        }

        self.changed = true;
    }
}

impl View for AdapterView {
    fn draw(&self, printer: &Printer) {
        for (i, adapter) in self.adapters.iter().enumerate() {
            printer.print_styled((0, i), adapter);
        }
    }

    fn layout(&mut self, _: Vec2) {
        self.changed = false;
    }

    fn required_size(&mut self, _: XY<usize>) -> XY<usize> {
        Vec2::new(50, self.adapters.len() + 1)
    }

    fn needs_relayout(&self) -> bool {
        self.changed
    }
}

pub struct PathView {
    paths: Vec<SpannedString<Style>>,
    changed: bool,
}

impl PathView {
    pub fn new() -> Self {
        Self {
            paths: Vec::new(),
            changed: true,
        }
    }

    fn format_path(&self, adapters: &[Adapter], path: &Path) -> SpannedString<Style> {
        let mut line = SpannedString::new();

        let adapter = &adapters[(path.in_adapter() - 1) as usize];
        let s = format!("{} / {}", path.in_adapter(), adapter.kind());
        line.append(format!("{:<20} ", s));
        line.append(format!("{:>10}  ", path.in_hop()));

        let adapter = &adapters[(path.out_adapter() - 1) as usize];
        let s = format!("{} / {}", path.out_adapter(), adapter.kind());
        line.append(format!("{:<20} ", s));
        line.append(format!("{:>10} ", path.out_hop()));

        line.append(format!("{:>2}", if path.pmps() { 1 } else { 0 }));

        line
    }

    pub fn clear(&mut self) {
        self.paths.clear();
        self.changed = true;
    }

    pub fn add_paths(&mut self, adapters: &[Adapter], paths: &Vec<Path>) {
        for path in paths {
            self.paths.push(self.format_path(adapters, path));
        }

        self.changed = true;
    }

    fn draw_headers(&self, printer: &Printer) {
        let mut line = SpannedString::new();
        line.append_styled(format!("{:<20} ", "In Adapter"), theme::dialog_label());
        line.append_styled(format!("{:>10}  ", "In HopID"), theme::dialog_label());
        line.append_styled(format!("{:<20} ", "Out Adapter"), theme::dialog_label());
        line.append_styled(format!("{:>10} ", "Out HopID"), theme::dialog_label());
        line.append_styled(format!("{:2}", "PM"), theme::dialog_label());
        printer.print_styled((0, 0), &line);
    }

    fn draw_path(&self, i: usize, path: &SpannedString<Style>, printer: &Printer) {
        printer.print_styled((0, i), path);
    }
}

impl View for PathView {
    fn draw(&self, printer: &Printer) {
        self.draw_headers(printer);

        for (i, path) in self.paths.iter().enumerate() {
            self.draw_path(i + 1, path, printer);
        }
    }

    fn layout(&mut self, _: Vec2) {
        self.changed = false;
    }

    fn required_size(&mut self, _: XY<usize>) -> XY<usize> {
        Vec2::new(68, self.paths.len() + 1)
    }

    fn needs_relayout(&self) -> bool {
        self.changed
    }
}
