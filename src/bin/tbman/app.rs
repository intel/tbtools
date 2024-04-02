// Thunderbolt/USB4 live device manager
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use crate::{
    theme,
    views::{AdapterView, PathView},
};
use cursive::{
    align::HAlign,
    direction::Orientation,
    event::{Event, EventResult, Key},
    theme::Style,
    utils::span::SpannedString,
    view::{Nameable, Resizable},
    views::{
        Dialog, DummyView, EditView, HideableView, Layer, LinearLayout, ListView, NamedView,
        OnEventView, ScrollView, SelectView, TextView, ThemedView,
    },
    Cursive, View,
};
use std::{io, thread};
use tbtools::{
    debugfs::{Adapter, BitField, BitFields, Name, Register},
    monitor,
    trace::{self, Entry},
    util, Device, Kind, Pdf, {self, ConfigSpace},
};

struct Command<'a> {
    key: &'a str,
    desc: &'a str,
    help: &'a str,
    menu: bool,
}

// Names that are part of the view hierarchy and can be found.
const DEVICES: &str = "main.devices";
const DETAILS: &str = "main.details";
const FOOTER: &str = "main.footer";

const DIALOG_DEVICES: &str = "dialog.devices";
const DIALOG_NO_DEVICES: &str = "dialog.no_devices";
const DIALOG_ADAPTERS: &str = "dialog.adapters";
const DIALOG_PATHS: &str = "dialog.paths";
const DIALOG_REGISTERS: &str = "dialog.registers";
const DIALOG_TMU: &str = "dialog.tmu";
const DIALOG_TRACE: &str = "dialog.trace";

const VIEW_ADAPTERS: &str = "view.adapters";
const VIEW_PATHS: &str = "view.paths";
const VIEW_REGISTERS: &str = "view.registers";
const VIEW_TMU: &str = "view.tmu";
const VIEW_TRACE: &str = "view.trace";
const VIEW_ENTRIES: &str = "view.entries";

const MAIN_COMMANDS: [Command; 12] = [
    Command {
        key: "q/ESC",
        desc: "Quit",
        help: "Exit the program or close a dialog",
        menu: true,
    },
    Command {
        key: "↑/k",
        desc: "Up",
        help: "Move up one device",
        menu: false,
    },
    Command {
        key: "↓/j",
        desc: "Down",
        help: "Move down one device",
        menu: false,
    },
    Command {
        key: "F1",
        desc: "Help",
        help: "Show this help dialog",
        menu: true,
    },
    Command {
        key: "F2",
        desc: "Auth",
        help: "Authorize or deauthorize device",
        menu: true,
    },
    Command {
        key: "F3",
        desc: "Trace enable/disable",
        help: "Enables and disables kernel driver tracing",
        menu: false,
    },
    Command {
        key: "F4",
        desc: "View trace",
        help: "View kernel driver trace entries",
        menu: false,
    },
    Command {
        key: "F5",
        desc: "Refresh",
        help: "Refresh screen or dialog",
        menu: true,
    },
    Command {
        key: "F6",
        desc: "Adapters",
        help: "Show device adapters",
        menu: true,
    },
    Command {
        key: "F7",
        desc: "Paths",
        help: "Show paths through device",
        menu: true,
    },
    Command {
        key: "F8",
        desc: "Regs",
        help: "Access device config spaces",
        menu: true,
    },
    Command {
        key: "F9",
        desc: "TMU",
        help: "TMU configuration",
        menu: true,
    },
];

fn set_footer(siv: &mut Cursive, commands: &[Command]) {
    let mut footer = SpannedString::new();

    for command in commands.iter().filter(|c| c.menu) {
        footer.append_styled(format!("{:>3}", command.key), theme::footer_key());
        footer.append_styled(format!("{:<5} ", command.desc), theme::footer_desc());
    }

    siv.call_on_name(FOOTER, |tv: &mut TextView| {
        tv.set_content(footer);
    });
}

fn build_help(siv: &mut Cursive, about: &str, commands: &[Command]) {
    let mut help = SpannedString::new();

    for command in commands.iter() {
        help.append_styled(format!("{:<8} ", command.key), theme::dialog_label());
        help.append(command.help.to_string());
        help.append("\n");
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(
                    LinearLayout::new(Orientation::Vertical)
                        .child(TextView::new(about))
                        .child(DummyView)
                        .child(TextView::new(help)),
                )
                .title("Help")
                .button("Close", |s| {
                    s.pop_layer();
                })
                .min_width(50),
            )
            .on_event('q', |s| {
                s.pop_layer();
            })
            .on_event(Key::Esc, |s| {
                s.pop_layer();
            }),
        ),
    ));
}

fn close_dialog(siv: &mut Cursive, dialog: &str) -> bool {
    if siv.find_name::<Dialog>(dialog).is_some() {
        siv.pop_layer();
        set_footer(siv, &MAIN_COMMANDS);
        return true;
    }
    false
}

fn close_any_dialog(siv: &mut Cursive) {
    let dialogs = vec![
        DIALOG_NO_DEVICES,
        DIALOG_ADAPTERS,
        DIALOG_PATHS,
        DIALOG_REGISTERS,
        DIALOG_TMU,
    ];

    for dialog in dialogs {
        if close_dialog(siv, dialog) {
            break;
        }
    }
}

fn authorize_device(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();

    if let Some(index) = devices.selected_id() {
        let (_, device) = devices.get_item_mut(index).unwrap();

        if device.is_host_router() {
            return;
        }

        if let Some(authorized) = device.authorized() {
            if authorized && device.domain().unwrap().deauthorization().unwrap_or(false) {
                if let Err(err) = tbtools::authorize_device(device, 0) {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Device de-authorization failed: {}",
                            err
                        ))),
                    ));
                }
            } else if !authorized {
                if let Err(err) = tbtools::authorize_device(device, 1) {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Device authorization failed: {}",
                            err
                        ))),
                    ));
                }
            }
        }
    }
}

fn devices_empty(siv: &mut Cursive) -> bool {
    let devices: &SelectView<Device> = &siv.find_name(DEVICES).unwrap();
    devices.len() == 0
}

fn update_adapter_view(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();

    if let Some(index) = devices.selected_id() {
        let device = devices.get_item_mut(index).unwrap().1;
        let sink = siv.cb_sink().clone();

        siv.call_on_name(VIEW_ADAPTERS, |av: &mut AdapterView| {
            if let Err(err) = device.read_adapters() {
                sink.send(Box::new(move |s: &mut Cursive| {
                    s.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read device adapters: {}",
                            err
                        ))),
                    ));
                }))
                .unwrap();
            } else if let Some(adapters) = device.adapters() {
                av.clear();
                av.add_adapters(adapters);
            }
        });
    }
}

fn build_adapters(siv: &mut Cursive) {
    const COMMANDS: [Command; 3] = [
        Command {
            key: "q/ESC",
            desc: "Close",
            help: "Close the dialog",
            menu: true,
        },
        Command {
            key: "F1",
            desc: "Help",
            help: "Show this help",
            menu: true,
        },
        Command {
            key: "F5",
            desc: "Refresh",
            help: "Refresh adapters from hardware",
            menu: true,
        },
    ];

    if devices_empty(siv) {
        return;
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(
                    ScrollView::new(AdapterView::new().with_name(VIEW_ADAPTERS)).max_height(25),
                )
                .button("Close", |s| {
                    close_dialog(s, DIALOG_ADAPTERS);
                })
                .title("Adapters")
                .title_position(HAlign::Left)
                .with_name(DIALOG_ADAPTERS),
            )
            .on_event(Key::F1, |s| {
                build_help(s, "Show adapters and their current states", &COMMANDS);
            })
            .on_event(Key::F5, update_adapter_view)
            .on_event('q', |s| {
                close_dialog(s, DIALOG_ADAPTERS);
            })
            .on_event(Key::Esc, |s| {
                close_dialog(s, DIALOG_ADAPTERS);
            }),
        ),
    ));

    set_footer(siv, &COMMANDS);
    update_adapter_view(siv);
}

fn update_path_view(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();

    if let Some(index) = devices.selected_id() {
        let device = devices.get_item_mut(index).unwrap().1;
        let sink = siv.cb_sink().clone();

        siv.call_on_name(VIEW_PATHS, |pv: &mut PathView| {
            if let Err(err) = device.read_adapters() {
                sink.send(Box::new(move |s: &mut Cursive| {
                    s.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read device adapters: {}",
                            err
                        ))),
                    ));
                }))
                .unwrap();
            } else if let Some(adapters) = device.adapters_mut() {
                pv.clear();

                let mut paths = Vec::new();
                for adapter in adapters.iter_mut() {
                    if adapter.read_paths().is_err() {
                        continue;
                    }

                    if let Some(p) = adapter.paths() {
                        p.iter().for_each(|p| paths.push(*p));
                    }
                }

                pv.add_paths(adapters, &paths);
            }
        });
    }
}

fn build_paths(siv: &mut Cursive) {
    const COMMANDS: [Command; 3] = [
        Command {
            key: "q/ESC",
            desc: "Close",
            help: "Close the dialog",
            menu: true,
        },
        Command {
            key: "F1",
            desc: "Help",
            help: "Show this help",
            menu: true,
        },
        Command {
            key: "F5",
            desc: "Refresh",
            help: "Refresh paths from hardware",
            menu: true,
        },
    ];

    if devices_empty(siv) {
        return;
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(
                    ScrollView::new(PathView::new().with_name(VIEW_PATHS)).max_height(20),
                )
                .button("Close", |s| {
                    close_dialog(s, DIALOG_PATHS);
                })
                .title("Paths")
                .title_position(HAlign::Left)
                .with_name(DIALOG_PATHS),
            )
            .on_event(Key::F1, |s| {
                build_help(s, "Show paths through this device", &COMMANDS);
            })
            .on_event(Key::F5, update_path_view)
            .on_event('q', |s| {
                close_dialog(s, DIALOG_PATHS);
            })
            .on_event(Key::Esc, |s| {
                close_dialog(s, DIALOG_PATHS);
            }),
        ),
    ));

    set_footer(siv, &COMMANDS);
    update_path_view(siv);
}

fn list_header(space: &ConfigSpace) -> String {
    match *space {
        ConfigSpace::Unknown => String::from(""),
        ConfigSpace::Router | ConfigSpace::Adapter => {
            String::from("Offset Relative CapID VSEC Value    Name")
        }
        ConfigSpace::Path => String::from("Offset Relative HopID Value    Name"),
        ConfigSpace::Counters => String::from("Offset Relative CounterID Value"),
    }
}

fn list_entry(space: &ConfigSpace, reg: &Register) -> SpannedString<Style> {
    let mut entry = SpannedString::new();

    let offset = format!("{:04x}", reg.offset());
    let relative_offset = format!("{:04}", reg.relative_offset());
    let cap_id = format!("{:x}", reg.cap_id());
    let vs_cap_id = format!("{:x}", reg.vs_cap_id());
    let value = format!("{:08x}", reg.value());
    let name = if let Some(name) = reg.name() {
        format!(" {}", name)
    } else {
        String::from("")
    };

    let line = match *space {
        ConfigSpace::Unknown => panic!(),
        ConfigSpace::Router | ConfigSpace::Adapter => {
            format!(
                "{:<6} {:<8} {:<5} {:<4} {}{}",
                offset, relative_offset, cap_id, vs_cap_id, value, name
            )
        }
        ConfigSpace::Path => {
            format!(
                "{:<6} {:<8} {:<5} {}{}",
                offset, relative_offset, cap_id, value, name
            )
        }
        ConfigSpace::Counters => {
            format!(
                "{:<6} {:<8} {:<9} {}",
                offset, relative_offset, cap_id, value,
            )
        }
    };

    if reg.is_changed() {
        entry.append_styled(format!("{}*", line), theme::register_changed());
    } else {
        entry.append(line);
    }

    entry
}

fn selected_space(siv: &mut Cursive) -> ConfigSpace {
    let spaces = siv.find_name::<SelectView<ConfigSpace>>("spaces").unwrap();
    *spaces.selection().unwrap()
}

fn selected_adapter<'a>(siv: &mut Cursive, device: &'a mut Device) -> &'a mut Adapter {
    let adapters = siv.find_name::<SelectView<u16>>("adapters").unwrap();
    let adapter = adapters.selection().unwrap();

    device.adapter_mut(*adapter).unwrap()
}

fn read_registers(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();

    if let Some(index) = devices.selected_id() {
        let device = devices.get_item_mut(index).unwrap().1;

        let space = selected_space(siv);
        let adapter = selected_adapter(siv, device);

        let hw_regs = match space {
            ConfigSpace::Unknown => None,

            ConfigSpace::Router => {
                if let Err(err) = device.read_registers() {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read device registers: {}",
                            err
                        ))),
                    ));
                    return;
                }

                device.registers()
            }

            ConfigSpace::Adapter => {
                if let Err(err) = adapter.read_registers() {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read adapter registers: {}",
                            err
                        ))),
                    ));
                    return;
                }

                adapter.registers()
            }

            ConfigSpace::Path => {
                if let Err(err) = adapter.read_paths() {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read adapter paths : {}",
                            err
                        ))),
                    ));
                    return;
                }

                adapter.path_registers()
            }

            ConfigSpace::Counters => {
                if let Err(err) = adapter.read_counters() {
                    siv.add_layer(ThemedView::new(
                        theme::dialog(),
                        Layer::new(Dialog::info(format!(
                            "Failed to read adapter counters : {}",
                            err
                        ))),
                    ));
                    return;
                }

                adapter.counter_registers()
            }
        };

        let headers: &mut TextView = &mut siv.find_name("headers").unwrap();
        let mut header = SpannedString::new();
        header.append_styled(list_header(&space), theme::dialog_label());
        headers.set_content(header);

        // Clear the existing views.
        let registers: &mut SelectView<Register> = &mut siv.find_name(VIEW_REGISTERS).unwrap();
        registers.clear();

        if let Some(hw_regs) = hw_regs {
            // We clone the hardware register for simplicity. Probably could use some fancy
            // RefCell<T> directly but for now every time a register needs to be written back to
            // the hardware, we should first update the actual register (device or adapter) before
            // calling the write_changed().
            registers.add_all(hw_regs.iter().map(|r| (list_entry(&space, r), r.clone())));

            let cb = registers.set_selection(0);
            cb(siv);
        }
    }
}

fn search_registers(siv: &mut Cursive) {
    let registers: &SelectView<Register> = &siv.find_name(VIEW_REGISTERS).unwrap();
    if registers.is_empty() {
        return;
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::new()
                    .title("Search")
                    .content(EditView::new().with_name("register_name").fixed_width(20))
                    .button("Search", |s| {
                        let search = s
                            .call_on_name("register_name", |ev: &mut EditView| {
                                ev.get_content().to_lowercase()
                            })
                            .unwrap();

                        let registers: &mut SelectView<Register> =
                            &mut s.find_name(VIEW_REGISTERS).unwrap();
                        let mut index = None;

                        for (i, (_, reg)) in registers.iter_mut().enumerate() {
                            if let Some(name) = reg.name() {
                                if name.to_lowercase().contains(&search) {
                                    index = Some(i);
                                    break;
                                }
                            }
                        }

                        if let Some(index) = index {
                            // update the scroll accordingly.
                            let cb = registers.set_selection(index);
                            cb(s);
                        }

                        s.pop_layer();
                    })
                    .button("Cancel", |s| {
                        s.pop_layer();
                    }),
            )
            .on_event(Key::Esc, |s| {
                s.pop_layer();
            }),
        ),
    ));
}

fn edit_register(siv: &mut Cursive) {
    let registers: &mut SelectView<Register> = &mut siv.find_name(VIEW_REGISTERS).unwrap();
    if let Some(reg) = registers.selection() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(
                OnEventView::new(
                    Dialog::new()
                        .title("Edit")
                        .content(
                            EditView::new()
                                .content(format!("{:08x}", reg.value()))
                                .max_content_width(8)
                                .with_name("register_value")
                                .fixed_width(20),
                        )
                        .button("Update", move |s| {
                            let registers: &mut SelectView<Register> =
                                &mut s.find_name(VIEW_REGISTERS).unwrap();
                            if registers.selected_id().is_none() {
                                return;
                            }
                            let index = registers.selected_id().unwrap();
                            let space = selected_space(s);

                            if let Some(value) = s
                                .call_on_name("register_value", |ev: &mut EditView| {
                                    util::parse_hex::<u32>(&ev.get_content())
                                })
                                .unwrap()
                            {
                                if let Some((entry, reg)) = registers.get_item_mut(index) {
                                    let devices: &mut SelectView<Device> =
                                        &mut s.find_name(DEVICES).unwrap();
                                    let index = devices.selected_id().unwrap();
                                    let device = devices.get_item_mut(index).unwrap().1;
                                    let offset = reg.offset() as u16;

                                    let hw_reg = match space {
                                        ConfigSpace::Unknown => panic!(),
                                        ConfigSpace::Router => {
                                            device.register_by_offset_mut(offset)
                                        }
                                        ConfigSpace::Adapter => {
                                            let adapter = selected_adapter(s, device);
                                            adapter.register_by_offset_mut(offset)
                                        }

                                        ConfigSpace::Path => {
                                            let adapter = selected_adapter(s, device);
                                            adapter.path_register_by_offset_mut(offset)
                                        }

                                        ConfigSpace::Counters => {
                                            let adapter = selected_adapter(s, device);
                                            adapter.counter_register_by_offset_mut(offset)
                                        }
                                    };

                                    if let Some(hw_reg) = hw_reg {
                                        hw_reg.set_value(value);
                                        reg.set_value(value);
                                        *entry = list_entry(&space, reg);
                                    }
                                }
                            }

                            s.pop_layer();
                        })
                        .button("Cancel", |s| {
                            s.pop_layer();
                        }),
                )
                .on_event(Key::Esc, |s| {
                    s.pop_layer();
                }),
            ),
        ));
    }
}

fn write_changed(siv: &mut Cursive) -> bool {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let index = devices.selected_id().unwrap();
    let device = devices.get_item_mut(index).unwrap().1;

    if selected_space(siv) == ConfigSpace::Router {
        if let Err(err) = device.write_changed() {
            siv.add_layer(ThemedView::new(
                theme::dialog(),
                Layer::new(Dialog::info(format!("Failed to write registers: {}", err,))),
            ));
            return false;
        }
    } else {
        let adapter = selected_adapter(siv, device);

        if let Err(err) = adapter.write_changed() {
            siv.add_layer(ThemedView::new(
                theme::dialog(),
                Layer::new(Dialog::info(format!(
                    "Failed to write adapter registers: {}",
                    err,
                ))),
            ));
            return false;
        }
    }

    true
}

fn write_registers(siv: &mut Cursive) {
    if write_changed(siv) {
        read_registers(siv);
    }
}

fn build_register_detail(label: &str, value: &str) -> impl View {
    let mut line = SpannedString::new();

    line.append_styled(format!("{:>16} ", label), theme::dialog_label());
    line.append(value);

    TextView::new(line)
}

fn build_field_detail(bitfields: &dyn BitFields<u32>, field: &BitField) -> impl View {
    let mut line = SpannedString::new();

    line.append(" ");
    line.append(format!(
        "[{:>02}:{:>02}] ",
        field.range().start(),
        field.range().end()
    ));

    let value = bitfields.field(field.name());

    line.append_styled(format!("{:>#10x} ", value), theme::field_value());

    line.append_styled(field.name().to_string(), theme::dialog_label());
    if let Some(short_name) = field.short_name() {
        line.append(" (");
        line.append_styled(short_name.to_string(), theme::field_shortname());
        line.append(")");
    }

    if let Some(value_name) = field.value_name(value) {
        line.append(" → ");
        line.append_styled(value_name.to_string(), theme::field_value());
    }

    TextView::new(line)
}

fn view_register(siv: &mut Cursive, reg: &Register) {
    let mut l = LinearLayout::vertical();

    l.add_child(build_register_detail(
        "Offset:",
        &format!("0x{:04x}", reg.offset()),
    ));
    l.add_child(build_register_detail(
        "Relative offset:",
        &format!("{}", reg.relative_offset()),
    ));
    let value = reg.value();
    l.add_child(build_register_detail("Hex:", &format!("0x{:08x}", value)));

    let values: [u8; 4] = [
        (value & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        ((value >> 16) & 0xff) as u8,
        ((value >> 24) & 0xff) as u8,
    ];

    let mut binary = String::new();
    binary.push_str(&format!("0b{:08b}", values[3]));
    binary.push_str(&format!(" {:08b}", values[2]));
    binary.push_str(&format!(" {:08b}", values[1]));
    binary.push_str(&format!(" {:08b}", values[0]));
    l.add_child(build_register_detail("Binary:", &binary));

    l.add_child(build_register_detail(
        "Char:",
        &util::bytes_to_ascii(&value.to_be_bytes()),
    ));

    let mut title = String::from("Register details");

    if let Some(name) = reg.name() {
        title.push_str(&format!(": {}", name));

        if let Some(fields) = reg.fields() {
            l.add_child(build_register_detail("Fields:", ""));
            l.add_child(DummyView);

            for field in fields {
                l.add_child(build_field_detail(reg, field));
            }
        }
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(l)
                    .button("Close", |s| {
                        s.pop_layer();
                    })
                    .title(title)
                    .title_position(HAlign::Left),
            )
            .on_event('q', |s| {
                s.pop_layer();
            })
            .on_event(Key::Esc, |s| {
                s.pop_layer();
            }),
        ),
    ));
}

fn build_registers(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let index = devices.selected_id();
    if index.is_none() {
        return;
    }
    let device = devices.get_item_mut(index.unwrap()).unwrap().1;

    let mut commands = vec![
        Command {
            key: "q/ESC",
            desc: "Close",
            help: "Close the dialog",
            menu: true,
        },
        Command {
            key: "↑/k",
            desc: "Up",
            help: "Move up one register",
            menu: false,
        },
        Command {
            key: "↓/j",
            desc: "Down",
            help: "Move down one register",
            menu: false,
        },
        Command {
            key: "Tab",
            desc: "Focus",
            help: "Change focus to another component",
            menu: false,
        },
        Command {
            key: "↵/ENTER",
            desc: "Select",
            help: "Read registers or view details of a selected register",
            menu: true,
        },
        Command {
            key: "F1",
            desc: "Help",
            help: "Show this help",
            menu: true,
        },
        Command {
            key: "F5",
            desc: "Refresh",
            help: "Read registers from hardware",
            menu: true,
        },
        Command {
            key: "f/F6",
            desc: "Search",
            help: "Search for given register",
            menu: true,
        },
    ];

    // Only allow writing if `CONFIG_USB4_DEBUGFS_WRITE=y` is set in the kernel configuration.
    if device.registers_writable() {
        commands.push(Command {
            key: "F7",
            desc: "Edit",
            help: "Edit selected register",
            menu: true,
        });
        commands.push(Command {
            key: "F8",
            desc: "Write",
            help: "Write changed registers back to the hardware",
            menu: true,
        });
    }

    if let Err(err) = device.read_adapters() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info(format!(
                "Device adapters read failed: {}",
                err,
            ))),
        ));
        return;
    };

    let spaces = SelectView::new()
        .popup()
        .on_submit(|s, space| {
            read_registers(s);

            // Enable/disable adapters view depending on the config space.
            s.call_on_name("adapters_visible", |v: &mut HideableView<ListView>| {
                if *space == ConfigSpace::Router || *space == ConfigSpace::Unknown {
                    v.hide();
                } else {
                    v.unhide();
                }
            });
        })
        .item("None", ConfigSpace::Unknown)
        .item("Router", ConfigSpace::Router)
        .item("Adapter", ConfigSpace::Adapter)
        .item("Path", ConfigSpace::Path)
        .item("Counters", ConfigSpace::Counters);

    let mut adapters = SelectView::new()
        .popup()
        .on_submit(|s, _| read_registers(s));

    if let Some(all_adapters) = device.adapters_mut() {
        for adapter in all_adapters {
            if adapter.is_valid() {
                if adapter.is_lane0() {
                    adapters.add_item(format!("{} / Lane 0", adapter.adapter()), adapter.adapter());
                } else if adapter.is_lane1() {
                    adapters.add_item(format!("{} / Lane 1", adapter.adapter()), adapter.adapter());
                } else {
                    adapters.add_item(
                        format!("{} / {}", adapter.adapter(), adapter.kind()),
                        adapter.adapter(),
                    );
                }
            }
        }
    }

    let headers = TextView::new("").with_name("headers");

    let registers = OnEventView::new(
        SelectView::<Register>::new()
            .on_submit(view_register)
            .with_name(VIEW_REGISTERS),
    )
    .on_pre_event_inner('k', |o, _| {
        let cb = o.get_mut().select_up(1);
        Some(EventResult::Consumed(Some(cb)))
    })
    .on_pre_event_inner('j', |o, _| {
        let cb = o.get_mut().select_down(1);
        Some(EventResult::Consumed(Some(cb)))
    });

    set_footer(siv, &commands);

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::new()
                    .content(
                        LinearLayout::vertical()
                            .child(
                                ListView::new().child(
                                    "Config space",
                                    spaces.with_name("spaces").max_width(15),
                                ),
                            )
                            .child(
                                HideableView::new(ListView::new().child(
                                    "Adapter",
                                    adapters.with_name("adapters").max_width(15),
                                ))
                                .hidden()
                                .with_name("adapters_visible"),
                            )
                            .child(DummyView)
                            .child(headers)
                            .child(ScrollView::new(registers).max_height(15)),
                    )
                    .button("Close", |s| {
                        close_dialog(s, DIALOG_REGISTERS);
                    })
                    .title("Registers")
                    .title_position(HAlign::Left)
                    .with_name(DIALOG_REGISTERS),
            )
            .on_event(Key::F1, move |s| {
                build_help(s, "Access device configuration spaces", &commands);
            })
            .on_event(Key::F5, read_registers)
            .on_event(Key::F6, search_registers)
            .on_event('f', search_registers)
            .on_event(Key::F7, edit_register)
            .on_event(Key::F8, write_registers)
            .on_event('q', |s| {
                close_dialog(s, DIALOG_REGISTERS);
            })
            .on_event(Key::Esc, |s| {
                close_dialog(s, DIALOG_REGISTERS);
            }),
        )
        .fixed_height(22)
        .min_width(60),
    ));
}

fn adapter_tmu_is_enhanced(adapter: &Adapter) -> bool {
    if let Some(reg) = adapter.register_by_name("TMU_ADP_CS_8") {
        return reg.flag("EUDM");
    }
    false
}

fn adapter_tmu_is_unidirectional(adapter: &Adapter) -> bool {
    if let Some(reg) = adapter.register_by_name("TMU_ADP_CS_3") {
        return reg.flag("UDM");
    }
    false
}

fn build_tmu_detail(label: &str, value: &str) -> impl View {
    let mut line = SpannedString::new();

    line.append_styled(format!("{:>25} ", label), theme::dialog_label());
    line.append(value);

    TextView::new(line)
}

fn read_tmu(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let index = devices.selected_id().unwrap();
    let device = devices.get_item_mut(index).unwrap().1;

    if let Err(err) = device.read_registers() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info(format!(
                "Failed to read device TMU registers: {}",
                err
            ))),
        ));
        return;
    }

    if let Err(err) = device.read_adapters() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info(format!(
                "Failed to read device adapters: {}",
                err
            ))),
        ));
        return;
    }

    // For device routers need to find the parent device too to be able to figure out the
    // unidirectional configuration.
    let parent = if device.is_device_router() {
        if let Some(mut parent) = device.parent() {
            if let Err(err) = parent.read_registers() {
                siv.add_layer(ThemedView::new(
                    theme::dialog(),
                    Layer::new(Dialog::info(format!(
                        "Failed to read parent device: {}",
                        err
                    ))),
                ));
                return;
            }
            if let Err(err) = parent.read_adapters() {
                siv.add_layer(ThemedView::new(
                    theme::dialog(),
                    Layer::new(Dialog::info(format!(
                        "Failed to read parent device adapters: {}",
                        err
                    ))),
                ));
                return;
            }
            Some(parent)
        } else {
            None
        }
    } else {
        None
    };

    siv.call_on_name(VIEW_TMU, |l: &mut LinearLayout| {
        l.clear();

        let reg = device.register_by_name("TMU_RTR_CS_0").unwrap();
        let ucap = reg.flag("UCAP");
        let enhanced = if let Some(usb4_version) = device.usb4_version() {
            usb4_version.major >= 2
        } else {
            false
        };

        let freq = reg.field("Freq Measurement Window");

        let reg = device.register_by_name("TMU_RTR_CS_3").unwrap();
        let rate = reg.field("TSPacketInterval");

        if device.is_device_router() {
            if let Some(parent) = parent {
                if let Some(adapter) = device.upstream_adapter() {
                    if let Some(upstream_adapter) = device.adapter(adapter) {
                        let reg = parent.register_by_name("TMU_RTR_CS_3").unwrap();
                        let parent_rate = reg.field("TSPacketInterval");
                        if enhanced && adapter_tmu_is_enhanced(upstream_adapter) {
                            l.add_child(build_tmu_detail(
                                "TMU mode:",
                                "Enhanced uni-directional, MedRes",
                            ));
                        } else if ucap && adapter_tmu_is_unidirectional(upstream_adapter) {
                            if parent_rate == 1000 {
                                l.add_child(build_tmu_detail(
                                    "TMU mode:",
                                    "Uni-directional, LowRes",
                                ));
                            } else if parent_rate == 16 {
                                l.add_child(build_tmu_detail("TMU mode:", "Uni-directional, HiFi"));
                            }
                        } else if rate > 0 {
                            l.add_child(build_tmu_detail("TMU mode:", "Bi-directional, HiFi"));
                        } else {
                            l.add_child(build_tmu_detail("TMU mode:", "Off"));
                        }
                    }
                }
            }
        }

        l.add_child(build_tmu_detail(
            "TSPacketInterval:",
            &format!("{} μs", rate),
        ));
        l.add_child(build_tmu_detail(
            "Freq measurement window:",
            &format!("{}", freq),
        ));

        let reg = device.register_by_name("TMU_RTR_CS_15").unwrap();

        let freq_avg = reg.field("FreqAvgConst");
        l.add_child(build_tmu_detail("FreqAvgConst:", &format!("{}", freq_avg)));

        let delay_avg = reg.field("DelayAvgConst");
        l.add_child(build_tmu_detail(
            "DelayAvgConst:",
            &format!("{}", delay_avg),
        ));

        let offset_avg = reg.field("OffsetAvgConst");
        l.add_child(build_tmu_detail(
            "OffsetAvgConst:",
            &format!("{}", offset_avg),
        ));

        let error_avg = reg.field("ErrorAvgConst");
        l.add_child(build_tmu_detail(
            "ErrorAvgConst:",
            &format!("{}", error_avg),
        ));

        if enhanced {
            let reg = device.register_by_name("TMU_RTR_CS_18").unwrap();
            let delta_avg = reg.field("DeltaAvgConst");

            l.add_child(build_tmu_detail(
                "DeltaAvgConst:",
                &format!("{}", delta_avg),
            ));
        }
    });
}

fn build_tmu(siv: &mut Cursive) {
    const COMMANDS: [Command; 3] = [
        Command {
            key: "q/ESC",
            desc: "Close",
            help: "Close the dialog",
            menu: true,
        },
        Command {
            key: "F1",
            desc: "Help",
            help: "Show this help",
            menu: true,
        },
        Command {
            key: "F5",
            desc: "Refresh",
            help: "Re-read TMU configuration from hardware",
            menu: true,
        },
    ];

    if devices_empty(siv) {
        return;
    }

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(LinearLayout::vertical().with_name(VIEW_TMU))
                    .button("Close", |s| {
                        close_dialog(s, DIALOG_TMU);
                    })
                    .title("TMU")
                    .title_position(HAlign::Left)
                    .with_name(DIALOG_TMU),
            )
            .on_event(Key::F1, |s| {
                build_help(s, "Show router TMU configuration", &COMMANDS);
            })
            .on_event(Key::F5, read_tmu)
            .on_event('q', |s| {
                close_dialog(s, DIALOG_TMU);
            })
            .on_event(Key::Esc, |s| {
                close_dialog(s, DIALOG_TMU);
            }),
        ),
    ));

    read_tmu(siv);
    set_footer(siv, &COMMANDS);
}

fn update_title(siv: &mut Cursive) {
    siv.call_on_name(DIALOG_DEVICES, |d: &mut Dialog| {
        let mut title = SpannedString::new();
        title.append("⚡ Devices");
        if trace::enabled() {
            title.append(" ");
            title.append_styled("●", theme::trace_indicator());
        }
        d.set_title(title)
    });
}

fn enable_trace(siv: &mut Cursive) {
    if !trace::supported() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info("Kernel driver tracing is not supported")),
        ));
        return;
    }

    if trace::enabled() {
        if let Err(err) = trace::disable() {
            siv.add_layer(ThemedView::new(
                theme::dialog(),
                Layer::new(Dialog::info(format!("Failed to disable tracing: {}", err))),
            ));
        } else {
            update_title(siv);
        }
    } else if let Err(err) = trace::enable() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info(format!("Failed to enable tracing: {}", err))),
        ));
    } else {
        update_title(siv);
    }
}

fn build_trace_detail(label: &str, value: &str) -> impl View {
    let mut line = SpannedString::new();

    line.append_styled(format!("{:<14} ", label), theme::dialog_label());
    line.append(value);

    TextView::new(line)
}

fn fetch_device_registers(entry: &trace::Entry, device: &mut Device) -> io::Result<()> {
    match entry.cs() {
        Some(ConfigSpace::Adapter) => {
            device.read_registers_cached()?;
            device.read_adapters_cached()?;
        }

        Some(ConfigSpace::Path) => {
            if let Some(adapter_num) = entry.adapter_num() {
                device.read_registers_cached()?;
                device.read_adapters_cached()?;
                if let Some(adapter) = device.adapter_mut(adapter_num) {
                    adapter.read_paths_cached()?;
                }
            }
        }

        Some(ConfigSpace::Counters) => {
            if let Some(adapter_num) = entry.adapter_num() {
                device.read_registers_cached()?;
                device.read_adapters_cached()?;
                if let Some(adapter) = device.adapter_mut(adapter_num) {
                    adapter.read_counters()?;
                }
            }
        }

        Some(ConfigSpace::Router) => {
            device.read_registers_cached()?;
        }

        _ => {
            if (entry.pdf() == Pdf::HotPlugEvent || entry.event().is_some())
                && entry.adapter_num().is_some()
            {
                device.read_registers_cached()?;
                device.read_adapters_cached()?;
            }
        }
    }

    Ok(())
}

fn fetch_device_register(
    entry: &trace::Entry,
    device: &Device,
    offset: u16,
    value: u32,
) -> Option<impl BitFields<u32> + Name> {
    // Use the register metadata to print the details if it is available.
    if let Some(register) = match entry.cs() {
        Some(ConfigSpace::Adapter) => {
            if let Some(adapter_num) = entry.adapter_num() {
                if let Some(adapter) = device.adapter(adapter_num) {
                    adapter.register_by_offset(offset)
                } else {
                    None
                }
            } else {
                None
            }
        }

        Some(ConfigSpace::Path) => {
            if let Some(adapter_num) = entry.adapter_num() {
                if let Some(adapter) = device.adapter(adapter_num) {
                    adapter.path_register_by_offset(offset)
                } else {
                    None
                }
            } else {
                None
            }
        }

        Some(ConfigSpace::Router) => device.register_by_offset(offset),

        _ => None,
    } {
        // Clone it so that we can fill in the value and use the field metadata too without
        // changing the actual contents.
        let mut register = register.clone();
        register.set_value(value);

        return Some(register);
    }

    None
}

fn view_packet(siv: &mut Cursive, entry: &Entry) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();

    let mut device = devices
        .iter_mut()
        .find(|(_, d)| d.domain_index() == entry.domain_index() && d.route() == entry.route())
        .map(|(_, d)| d);

    if let Some(ref mut device) = device {
        // Don't really care if this fails, it is just additional non-essential information.
        let _ = fetch_device_registers(entry, device);
    }

    let mut header = LinearLayout::horizontal();

    let mut left = LinearLayout::vertical();
    left.add_child(build_trace_detail("CPU:", &entry.cpu().to_string()));
    left.add_child(build_trace_detail("Task:", entry.task()));
    left.add_child(build_trace_detail("PID:", &entry.pid().to_string()));
    left.add_child(build_trace_detail("Function:", entry.function()));
    left.add_child(build_trace_detail("Size:", &entry.size().to_string()));
    left.add_child(build_trace_detail(
        "Dropped:",
        if entry.dropped() { "Yes" } else { "No" },
    ));
    header.add_child(left);

    header.add_child(DummyView);

    let mut right = LinearLayout::vertical();
    right.add_child(build_trace_detail("PDF:", &entry.pdf().to_string()));
    if let Some(cs) = entry.cs() {
        right.add_child(build_trace_detail("Config space:", &cs.to_string()));
    }
    right.add_child(build_trace_detail(
        "Domain:",
        &entry.domain_index().to_string(),
    ));
    right.add_child(build_trace_detail(
        "Route:",
        &format!("{:x}", entry.route()),
    ));

    if let Some(adapter_num) = entry.adapter_num() {
        let mut adapter_details = format!("{}", adapter_num);
        if let Some(ref device) = device {
            if let Some(adapter) = device.adapter(adapter_num) {
                adapter_details.push_str(&format!(" / {}", adapter.kind()));
            }
        }
        right.add_child(build_trace_detail("Adapter:", &adapter_details));
    }

    header.add_child(right);

    let mut data = LinearLayout::vertical();

    if let Some(packet) = entry.packet() {
        let mut data_address = packet.data_address().unwrap_or(0);
        let data_start = packet.data_start().unwrap_or(0);

        for (i, f) in packet
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| (i as u16, f))
        {
            let mut line = SpannedString::new();

            line.append(format!("0x{:02x}", i));
            // Add the offset inside packet if known.
            if packet.data().is_some() && i >= data_start {
                line.append("/");
                line.append_styled(format!("{:04x} ", data_address), theme::field_offset());
            } else {
                line.append("      ");
            }

            let d = f.value();

            line.append(format!("0x{:08x} ", d));

            line.append(format!("0b{:08b}", (d >> 24) & 0xff));
            line.append(format!(" {:08b}", (d >> 16) & 0xff));
            line.append(format!(" {:08b}", (d >> 8) & 0xff));
            line.append(format!(" {:08b} ", d & 0xff));

            line.append(util::bytes_to_ascii(&d.to_be_bytes()));
            line.append(" ");

            if packet.data().is_some() && i >= data_start {
                if let Some(ref device) = device {
                    if let Some(reg) = fetch_device_register(entry, device, data_address, d) {
                        if let Some(name) = f.name() {
                            line.append(name);
                        }

                        data.add_child(TextView::new(line));

                        if let Some(bitfields) = reg.fields() {
                            for bf in bitfields {
                                data.add_child(build_field_detail(&reg, bf));
                            }
                        }

                        data_address += 1;
                        continue;
                    }
                }

                data_address += 1;
            }

            if let Some(name) = f.name() {
                line.append(name);
            }

            data.add_child(TextView::new(line));

            if let Some(bitfields) = f.fields() {
                for bf in bitfields {
                    data.add_child(build_field_detail(f, bf));
                }
            }
        }
    }

    let title = format!(
        "Packet @{}.{:06} ",
        entry.timestamp().tv_sec(),
        entry.timestamp().tv_usec()
    );

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(
                    LinearLayout::vertical()
                        .child(header)
                        .child(DummyView)
                        .child(ScrollView::new(data).max_height(15)),
                )
                .button("Close", |s| {
                    s.pop_layer();
                })
                .title(title)
                .title_position(HAlign::Left),
            )
            .on_event('q', |s| {
                s.pop_layer();
            })
            .on_event(Key::Esc, |s| {
                s.pop_layer();
            }),
        ),
    ));
}

fn trace_entry(entry: &Entry) -> SpannedString<Style> {
    let mut line = SpannedString::new();

    if entry.dropped() {
        line.append_styled("!", theme::trace_dropped());
    } else {
        line.append(" ");
    }

    let timestamp = format!(
        "{:6}.{:06} ",
        entry.timestamp().tv_sec(),
        entry.timestamp().tv_usec()
    );
    line.append(timestamp);
    line.append(format!("{:8} ", &entry.function()));
    let mut pdf = entry.pdf().to_string();
    match entry.pdf() {
        Pdf::ReadRequest | Pdf::WriteRequest | Pdf::ReadResponse | Pdf::WriteResponse => {
            pdf.push_str(" / ");
            pdf.push_str(&entry.cs().unwrap().to_string());
        }

        _ => (),
    }
    line.append(format!("{:25} ", pdf));
    line.append(format!("{:<6} ", entry.domain_index()));
    line.append(format!("{:<10x} ", entry.route()));

    if let Some(adapter_num) = entry.adapter_num() {
        line.append(format!("{:<2} ", adapter_num));
    }

    line
}

fn read_entries(siv: &mut Cursive) {
    let entries: &mut SelectView<Entry> = &mut siv.find_name(VIEW_ENTRIES).unwrap();

    entries.clear();

    if let Ok(trace_buf) = trace::live_buffer() {
        for entry in trace_buf {
            entries.add_item(trace_entry(&entry), entry);
        }

        // Force update of the details view.
        let cb = entries.set_selection(0);
        cb(siv);
    }
}

fn clear_trace(siv: &mut Cursive) {
    if trace::clear().is_ok() {
        let entries: &mut SelectView<Entry> = &mut siv.find_name(VIEW_ENTRIES).unwrap();

        entries.clear();
    }
}

fn jump_trace(siv: &mut Cursive) {
    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::new()
                    .title("Jump to timestamp")
                    .content(EditView::new().with_name("seconds").fixed_width(20))
                    .button("Jump", |s| {
                        if let Some(seconds) = s
                            .call_on_name("seconds", |ev: &mut EditView| {
                                util::parse_number::<i64>(&ev.get_content())
                            })
                            .unwrap()
                        {
                            let entries: &mut SelectView<Entry> =
                                &mut s.find_name(VIEW_ENTRIES).unwrap();
                            let mut index = None;

                            for (i, (_, entry)) in entries.iter().enumerate() {
                                if entry.timestamp().tv_sec() == seconds {
                                    index = Some(i);
                                    break;
                                }
                            }

                            if let Some(index) = index {
                                let cb = entries.set_selection(index);
                                cb(s);
                            }

                            s.pop_layer();
                        }
                    })
                    .button("Cancel", |s| {
                        s.pop_layer();
                    }),
            )
            .on_event(Key::Esc, |s| {
                s.pop_layer();
            }),
        ),
    ));
}

fn build_trace(siv: &mut Cursive) {
    if !trace::supported() {
        siv.add_layer(ThemedView::new(
            theme::dialog(),
            Layer::new(Dialog::info("Kernel driver tracing is not supported")),
        ));
        return;
    }

    const COMMANDS: [Command; 6] = [
        Command {
            key: "q/ESC",
            desc: "Close",
            help: "Close the dialog",
            menu: true,
        },
        Command {
            key: "↵/ENTER",
            desc: "View",
            help: "View details of the selected trace entry",
            menu: true,
        },
        Command {
            key: "F1",
            desc: "Help",
            help: "Show this help",
            menu: true,
        },
        Command {
            key: "F2",
            desc: "Clear",
            help: "Clear the trace buffer",
            menu: true,
        },
        Command {
            key: "F5",
            desc: "Refresh",
            help: "Re-read the trace buffer",
            menu: true,
        },
        Command {
            key: "F6",
            desc: "Jump",
            help: "Jump to given timestamp",
            menu: true,
        },
    ];

    let mut header = SpannedString::new();
    header.append_styled(
        " Timestamp     Function PDF / CS                  Domain Route      Adapter",
        theme::dialog_label(),
    );
    let headers = TextView::new(header);

    let entries = OnEventView::new(
        SelectView::<Entry>::new()
            .on_submit(view_packet)
            .with_name(VIEW_ENTRIES),
    )
    .on_pre_event_inner('k', |e, _| {
        let cb = e.get_mut().select_up(1);
        Some(EventResult::Consumed(Some(cb)))
    })
    .on_pre_event_inner('j', |o, _| {
        let cb = o.get_mut().select_down(1);
        Some(EventResult::Consumed(Some(cb)))
    });

    siv.add_layer(ThemedView::new(
        theme::dialog(),
        Layer::new(
            OnEventView::new(
                Dialog::around(
                    LinearLayout::vertical()
                        .child(headers)
                        .child(ScrollView::new(entries).max_height(20))
                        .with_name(VIEW_TRACE),
                )
                .button("Close", |s| {
                    close_dialog(s, DIALOG_TRACE);
                })
                .title("Trace")
                .title_position(HAlign::Left)
                .with_name(DIALOG_TRACE),
            )
            .on_event(Key::F1, |s| {
                build_help(s, "View system live tracing buffer", &COMMANDS);
            })
            .on_event(Key::F2, clear_trace)
            .on_event(Key::F5, read_entries)
            .on_event('q', |s| {
                close_dialog(s, DIALOG_TRACE);
            })
            .on_event('j', jump_trace)
            .on_event(Key::F6, jump_trace)
            .on_event(Key::Esc, |s| {
                close_dialog(s, DIALOG_TRACE);
            }),
        ),
    ));

    read_entries(siv);

    set_footer(siv, &COMMANDS);
}

fn build_detail(label: &str, value: String) -> impl View {
    let mut line = SpannedString::new();

    line.append_styled(format!("{:>14} ", label), theme::label());
    line.append(value);

    TextView::new(line)
}

fn build_details(siv: &mut Cursive, device: &Device) {
    // Update the textview with the currently selected item.
    siv.call_on_name(DETAILS, |l: &mut LinearLayout| {
        l.clear();

        l.add_child(build_detail(
            "Domain:",
            format!("{}", device.domain_index()),
        ));
        l.add_child(build_detail("Route:", format!("{:x}", device.route())));
        l.add_child(build_detail("Vendor:", format!("{:04x}", device.vendor())));

        if let Some(vendor_name) = device.vendor_name() {
            l.add_child(build_detail("Vendor name:", vendor_name));
        }

        l.add_child(build_detail("Product:", format!("{:04x}", device.device())));

        if let Some(model_name) = device.device_name() {
            l.add_child(build_detail("Model name:", model_name));
        }

        if let Some(unique_id) = device.unique_id() {
            l.add_child(build_detail("UUID", unique_id));
        }

        if let Some(generation) = device.generation() {
            let generation = match generation {
                1..=3 => format!("Thunderbolt {}", generation),
                4 => {
                    let version = device.usb4_version().unwrap();
                    format!("USB4 {}.{}", version.major, version.minor)
                }
                _ => String::from("Unknown"),
            };
            l.add_child(build_detail("Generation", generation));
        }

        if let Some(nvm_version) = device.nvm_version() {
            l.add_child(build_detail(
                "NVM version:",
                format!("{:x}.{:x}", nvm_version.major, nvm_version.minor),
            ));
        }

        // Device router
        if device.is_device_router() {
            if let Some(rx_speed) = device.rx_speed() {
                if let Some(rx_lanes) = device.rx_lanes() {
                    if let Some(tx_speed) = device.tx_speed() {
                        if let Some(tx_lanes) = device.tx_lanes() {
                            l.add_child(build_detail(
                                "Speed (Rx/Tx):",
                                format!("{}/{} Gb/s", rx_speed * rx_lanes, tx_speed * tx_lanes),
                            ));
                        }
                    }
                }
            }

            if let Some(authorized) = device.authorized() {
                let mut line = SpannedString::new();

                line.append_styled(format!("{:>14} ", "Authorized: "), theme::label());
                if authorized {
                    line.append_styled("Yes", theme::authorized());
                } else {
                    line.append("No");
                }

                l.add_child(TextView::new(line));
            }
        }

        l.add_child(build_detail("Kernel name:", device.kernel_name()));

        l.add_child(build_detail(
            "sysfs path:",
            device.sysfs_path().as_path().to_str().unwrap().to_string(),
        ));

        l.add_child(build_detail(
            "debugfs path:",
            device
                .debugfs_path()
                .as_path()
                .to_str()
                .unwrap()
                .to_string(),
        ));
    });
}

fn refresh_devices(siv: &mut Cursive) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    // TODO: Should we stop the monitor thread before doing this?
    devices.clear();

    let sink = siv.cb_sink().clone();

    tbtools::find_devices(None)
        .unwrap()
        .into_iter()
        .filter(|d| d.kind() == Kind::Router)
        .for_each(|d| {
            sink.send(Box::new(move |s: &mut Cursive| {
                add_device(s, d);
            }))
            .unwrap();
        });
}

fn build_devices() -> impl View {
    let mut devices: SelectView<Device> = SelectView::new();

    devices.set_on_select(build_details);
    let event = OnEventView::new(NamedView::new(DEVICES, devices))
        .on_pre_event_inner('k', |o, _| {
            let cb = o.get_mut().select_up(1);
            Some(EventResult::Consumed(Some(cb)))
        })
        .on_pre_event_inner('j', |o, _| {
            let cb = o.get_mut().select_down(1);
            Some(EventResult::Consumed(Some(cb)))
        })
        .on_event(Key::F2, authorize_device)
        .on_event(Key::F3, enable_trace)
        .on_event(Key::F4, build_trace)
        .on_event(Key::F5, refresh_devices)
        .on_event(Key::F6, build_adapters)
        .on_event(Key::F7, build_paths)
        .on_event(Key::F8, build_registers)
        .on_event(Key::F9, build_tmu);

    ScrollView::new(event)
}

fn build_device_list(siv: &mut Cursive) {
    let devices = build_devices();
    let footer = ThemedView::new(
        theme::footer(),
        Layer::new(TextView::new("").center().with_name(FOOTER)),
    );

    siv.add_fullscreen_layer(ThemedView::new(
        theme::device_list(),
        Layer::new(
            LinearLayout::new(Orientation::Vertical)
                .child(
                    Dialog::around(LinearLayout::new(Orientation::Vertical).child(devices))
                        .with_name(DIALOG_DEVICES)
                        .full_width()
                        .min_height(10),
                )
                .child(
                    Dialog::around(
                        LinearLayout::new(Orientation::Vertical)
                            .child(LinearLayout::new(Orientation::Vertical).with_name(DETAILS))
                            .full_width(),
                    )
                    .title("Details")
                    .full_height(),
                )
                .child(footer),
        ),
    ));

    update_title(siv);
    set_footer(siv, &MAIN_COMMANDS);
}

fn add_keys(siv: &mut Cursive) {
    siv.add_global_callback('q', |s| s.quit());
    siv.add_global_callback(Event::Key(Key::Esc), |s| s.quit());
    siv.add_global_callback(Event::Key(Key::F1), |s| {
        build_help(
            s,
            "Thunderbolt/USB4 live device manager and monitor.",
            &MAIN_COMMANDS,
        );
    });
}

fn device_name(device: &Device) -> String {
    let mut name = device.name();

    if let Some(device_name) = device.device_name() {
        name.push_str(&format!(" {}", device_name));
    }

    name
}

fn add_device(siv: &mut Cursive, device: Device) {
    close_dialog(siv, DIALOG_NO_DEVICES);

    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let selected = devices.selection();

    devices.add_item(device_name(&device), device);
    // Make sure it lands in the correct position.
    devices.sort();

    // If there is device selected, move the selection back to that device. If it is the first
    // device then update the details view now.
    let cb = if let Some(device) = selected {
        let index = devices.iter().position(|d| *d.1 == *device).unwrap();
        devices.set_selection(index)
    } else {
        devices.set_selection(0)
    };

    cb(siv);
}

fn remove_device(siv: &mut Cursive, device: Device) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let id = devices.selected_id();
    let index = devices.iter().position(|(_, d)| d == &device);

    if let Some(index) = index {
        let cb = devices.remove_item(index);
        // This one updates the details view to the next/prev device if any
        cb(siv);

        // If if the removed device was the same as selected one close any dialogs that may still
        // refer to it.
        if id == Some(index) {
            close_any_dialog(siv);
        }
    }

    // Clear the details view if there are no devices in the list
    if devices.is_empty() {
        close_any_dialog(siv);

        let l: &mut LinearLayout = &mut siv.find_name(DETAILS).unwrap();
        l.clear();
    }
}

fn update_device_list(siv: &mut Cursive, device: Device) {
    let devices: &mut SelectView<Device> = &mut siv.find_name(DEVICES).unwrap();
    let id = devices.selected_id();
    let index = devices.iter().position(|(_, d)| d == &device);

    if let Some(index) = index {
        devices.remove_item(index);
        devices.insert_item(index, device_name(&device), device);
        if id == Some(index) {
            let cb = devices.set_selection(index);
            cb(siv);
        }
    }
}

fn update_device(siv: &mut Cursive, device: Device) {
    update_device_list(siv, device);
    // Make sure to update the open adapter or path views to reflect the changed device.
    update_adapter_view(siv);
    update_path_view(siv);
}

fn handle_event(siv: &mut Cursive, event: monitor::Event) {
    match event {
        monitor::Event::Add(device) => add_device(siv, device),
        monitor::Event::Change(device) => update_device(siv, device),
        monitor::Event::Remove(device) => remove_device(siv, device),
    }
}

fn start_monitor(siv: &mut Cursive) {
    let sink = siv.cb_sink().clone();

    thread::spawn(move || {
        let mut monitor = monitor::Builder::new()
            .unwrap()
            .kind(Kind::Router)
            .unwrap()
            .build()
            .unwrap();

        // Get the initial list of devices
        let mut n = 0;
        tbtools::find_devices(None)
            .unwrap()
            .into_iter()
            .filter(|d| d.kind() == Kind::Router)
            .for_each(|d| {
                n += 1;
                sink.send(Box::new(move |s: &mut Cursive| {
                    add_device(s, d);
                }))
                .unwrap();
            });

        // No devices, show info dialog
        if n == 0 {
            sink.send(Box::new(move |s: &mut Cursive| {
                s.add_layer(ThemedView::new(
                    theme::dialog(),
                    Layer::new(
                        Dialog::info("No Thunderbolt/USB4 devices found")
                            .with_name(DIALOG_NO_DEVICES),
                    ),
                ));
            }))
            .unwrap();
        }

        loop {
            match monitor.poll(None) {
                Err(_) => {
                    // Handle error
                    break;
                }
                Ok(res) if res => {
                    for event in monitor.iter_mut() {
                        sink.send(Box::new(move |s: &mut Cursive| {
                            handle_event(s, event);
                        }))
                        .unwrap();
                    }
                }
                Ok(_) => (),
            }
        }
    });
}

pub fn run() {
    let mut siv = cursive::default();

    build_device_list(&mut siv);
    add_keys(&mut siv);
    start_monitor(&mut siv);

    siv.run();
}
