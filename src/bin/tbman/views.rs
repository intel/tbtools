// Thunderbolt/USB4 live device manager
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use crate::theme;
use cursive::{
    direction::Direction,
    event::{Callback, Event, EventResult, Key},
    theme::Style,
    utils::span::SpannedString,
    view::{CannotFocus, View},
    Cursive, Printer, Vec2, With, XY,
};
use std::sync::Arc;
use tbtools::debugfs::{Adapter, BitFields, State, Type};

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

type OnEdit = dyn Fn(&mut Cursive, &str, usize) + Send + Sync;

/// EditView but only supports numeric input.
///
/// This is similar to Cursive [`EditView`](cursive::views::EditView) but instead of being generic this one
/// only allows numeric input in either binary, decimal or hexadecimal format. All the editing is
/// done in-place instead of having a separate submit mechanism.
pub struct NumberEditView {
    content: Arc<String>,
    base: usize,
    cursor: usize,
    chunk_size: Option<usize>,
    max_content_width: Option<usize>,
    on_edit: Option<Arc<OnEdit>>,
}

impl NumberEditView {
    fn new(base: usize, chunk_size: Option<usize>) -> Self {
        Self {
            content: Arc::new(String::new()),
            base,
            cursor: 0,
            chunk_size,
            max_content_width: None,
            on_edit: None,
        }
    }

    fn is_valid(&self, ch: char) -> bool {
        match self.base {
            2 => ch == '0' || ch == '1',
            10 => ch.is_ascii_digit(),
            16 => ch.is_ascii_hexdigit(),
            _ => false,
        }
    }

    pub fn bin() -> Self {
        Self::new(2, Some(8))
    }

    #[allow(unused)]
    pub fn dec() -> Self {
        Self::new(10, None)
    }

    pub fn hex() -> Self {
        Self::new(16, None)
    }

    pub fn set_max_content_width(&mut self, width: Option<usize>) {
        self.max_content_width = width;
    }

    pub fn max_content_width(self, width: usize) -> Self {
        self.with(|s| s.set_max_content_width(Some(width)))
    }

    pub fn get_content(&self) -> Arc<String> {
        Arc::clone(&self.content)
    }

    pub fn set_content(&mut self, content: String) {
        for ch in content.chars() {
            if !self.is_valid(ch) {
                panic!("Only numbers expected, got {}", ch);
            }
        }

        let len = content.len();

        self.content = Arc::new(content);
        self.set_cursor(len);
    }

    pub fn content(mut self, content: String) -> Self {
        self.set_content(content);
        self
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn set_cursor(&mut self, cursor: usize) {
        self.cursor = cursor;
    }

    pub fn set_on_edit<F>(&mut self, callback: F)
    where
        F: Fn(&mut Cursive, &str, usize) + 'static + Send + Sync,
    {
        self.on_edit = Some(Arc::new(callback));
    }

    pub fn on_edit<F>(self, callback: F) -> Self
    where
        F: Fn(&mut Cursive, &str, usize) + 'static + Send + Sync,
    {
        self.with(|v| v.set_on_edit(callback))
    }

    fn make_callback(&self) -> Option<Callback> {
        self.on_edit.clone().map(|cb| {
            let content = Arc::clone(&self.content);
            let base = self.base;

            Callback::from_fn(move |s| {
                cb(s, &content, base);
            })
        })
    }

    pub fn insert(&mut self, ch: char) -> EventResult {
        if let Some(width) = self.max_content_width {
            if self.content.len() >= width {
                return EventResult::Consumed(Some(Callback::dummy()));
            }
        }

        let ch = ch.to_ascii_lowercase();

        if self.is_valid(ch) {
            Arc::make_mut(&mut self.content).insert(self.cursor, ch);
            self.cursor += 1;

            let cb = self.make_callback().unwrap_or_else(Callback::dummy);
            return EventResult::Consumed(Some(cb));
        }

        EventResult::Ignored
    }

    pub fn remove(&mut self, offset: usize) -> EventResult {
        Arc::make_mut(&mut self.content).remove(self.cursor - offset);
        self.cursor -= offset;

        let cb = self.make_callback().unwrap_or_else(Callback::dummy);
        EventResult::Consumed(Some(cb))
    }
}

impl View for NumberEditView {
    fn draw(&self, printer: &Printer) {
        let (style, cursor) = if printer.focused {
            theme::edit_active()
        } else {
            theme::edit_inactive()
        };

        let mut line = SpannedString::new();

        let s: Vec<_> = self.content.chars().collect();

        if let Some(chunk_size) = self.chunk_size {
            let mut offset = 0;
            let mut x = 0;

            for chunk in s.chunks(chunk_size) {
                if x > 0 {
                    line.append(" ");
                }

                for ch in chunk {
                    let style = if offset == self.cursor() && printer.focused {
                        cursor
                    } else {
                        style
                    };
                    line.append_styled(format!("{}", ch), style);
                    offset += 1;
                }

                x += chunk.len();
            }
        } else {
            for (i, ch) in s.iter().enumerate() {
                let style = if i == self.cursor() && printer.focused {
                    cursor
                } else {
                    style
                };
                line.append_styled(format!("{}", ch), style);
            }
        }

        printer.print_styled((0, 0), &line);
    }

    fn take_focus(&mut self, _: Direction) -> Result<EventResult, CannotFocus> {
        Ok(EventResult::consumed())
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        match event {
            Event::Char(ch) => self.insert(ch),

            Event::Key(Key::Left) if self.cursor() > 0 => {
                let cursor = self.cursor() - 1;
                self.set_cursor(cursor);
                EventResult::Consumed(None)
            }

            Event::Key(Key::Right) if self.cursor() < self.content.len() => {
                let cursor = self.cursor() + 1;
                self.set_cursor(cursor);
                EventResult::Consumed(None)
            }

            Event::Key(Key::Home) => {
                self.set_cursor(0);
                EventResult::Consumed(None)
            }

            Event::Key(Key::End) => {
                self.set_cursor(self.content.len());
                EventResult::Consumed(None)
            }

            Event::Key(Key::Backspace) if self.cursor() > 0 => self.remove(1),

            Event::Key(Key::Del) if self.cursor() < self.content.len() => self.remove(0),

            _ => EventResult::Ignored,
        }
    }

    fn required_size(&mut self, _: XY<usize>) -> XY<usize> {
        let mut size = if let Some(width) = self.max_content_width {
            width
        } else {
            self.content.len()
        };

        if let Some(chunk_size) = self.chunk_size {
            size += size / chunk_size;
        }

        Vec2::new(size, 1)
    }
}
