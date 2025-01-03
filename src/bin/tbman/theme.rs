// Thunderbolt/USB4 live device manager
//
// Copyright (C) 2023, Intel Corporation
// Author: Mika Westerberg <mika.westerberg@linux.intel.com>

use cursive::{
    theme::{BaseColor, BorderStyle, ColorStyle, Effect, Palette, PaletteColor, Style, Theme},
    With,
};

pub fn device_list() -> Theme {
    Theme {
        shadow: false,
        borders: BorderStyle::Simple,
        palette: Palette::default().with(|palette| {
            palette[PaletteColor::Background] = BaseColor::Blue.dark();
            palette[PaletteColor::View] = BaseColor::Blue.dark();
            palette[PaletteColor::Primary] = BaseColor::White.dark();
            palette[PaletteColor::Secondary] = BaseColor::Black.dark();
            palette[PaletteColor::Tertiary] = BaseColor::Black.dark();
            palette[PaletteColor::TitlePrimary] = BaseColor::White.dark();
            palette[PaletteColor::TitleSecondary] = BaseColor::White.dark();
            palette[PaletteColor::Highlight] = BaseColor::Cyan.dark();
            palette[PaletteColor::HighlightInactive] = BaseColor::Cyan.dark();
            palette[PaletteColor::HighlightText] = BaseColor::Black.dark();
        }),
    }
}

pub fn footer() -> Theme {
    Theme {
        shadow: false,
        borders: BorderStyle::Simple,
        palette: Palette::default().with(|palette| {
            palette[PaletteColor::Background] = BaseColor::Black.dark();
            palette[PaletteColor::View] = BaseColor::Black.dark();
            palette[PaletteColor::Primary] = BaseColor::White.dark();
            palette[PaletteColor::Secondary] = BaseColor::Black.dark();
            palette[PaletteColor::Tertiary] = BaseColor::Black.dark();
            palette[PaletteColor::TitlePrimary] = BaseColor::White.dark();
            palette[PaletteColor::TitleSecondary] = BaseColor::White.dark();
            palette[PaletteColor::Highlight] = BaseColor::Cyan.dark();
            palette[PaletteColor::HighlightInactive] = BaseColor::Blue.dark();
            palette[PaletteColor::HighlightText] = BaseColor::Black.dark();
        }),
    }
}

pub fn dialog() -> Theme {
    Theme {
        shadow: true,
        borders: BorderStyle::Simple,
        palette: Palette::default().with(|palette| {
            palette[PaletteColor::Background] = BaseColor::White.dark();
            palette[PaletteColor::View] = BaseColor::White.dark();
            palette[PaletteColor::Primary] = BaseColor::Black.dark();
            palette[PaletteColor::Secondary] = BaseColor::Black.light();
            palette[PaletteColor::Tertiary] = BaseColor::Black.dark();
            palette[PaletteColor::TitlePrimary] = BaseColor::Blue.dark();
            palette[PaletteColor::TitleSecondary] = BaseColor::Blue.dark();
            palette[PaletteColor::Highlight] = BaseColor::Cyan.dark();
            palette[PaletteColor::HighlightInactive] = BaseColor::Cyan.dark();
            palette[PaletteColor::HighlightText] = BaseColor::Black.dark();
        }),
    }
}

pub fn footer_key() -> Style {
    Style::from(ColorStyle::new(BaseColor::White, BaseColor::Black))
}

pub fn footer_desc() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black, BaseColor::Cyan))
}

pub fn label() -> Style {
    Style::from(ColorStyle::new(BaseColor::White, BaseColor::Blue)).combine(Effect::Bold)
}

pub fn authorized() -> Style {
    Style::from(ColorStyle::new(BaseColor::Green, BaseColor::Blue)).combine(Effect::Bold)
}

pub fn dialog_label() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black, BaseColor::White)).combine(Effect::Bold)
}

pub fn edit_active() -> (Style, Style) {
    (
        Style::from(ColorStyle::new(BaseColor::Black, BaseColor::Cyan)),
        Style::from(ColorStyle::new(BaseColor::Cyan, BaseColor::Black)),
    )
}

pub fn edit_inactive() -> (Style, Style) {
    (
        Style::from(ColorStyle::new(BaseColor::Black.light(), BaseColor::Cyan)),
        Style::from(ColorStyle::new(BaseColor::Black.light(), BaseColor::Cyan)),
    )
}

pub fn adapter_not_implemented() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black.light(), BaseColor::White))
}

pub fn adapter_disabled() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black, BaseColor::Black.light()))
}

pub fn adapter_enabled() -> Style {
    Style::from(ColorStyle::new(BaseColor::White, BaseColor::Green))
}

pub fn adapter_training() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black, BaseColor::Yellow))
}

pub fn adapter_pm() -> Style {
    Style::from(ColorStyle::new(BaseColor::Black, BaseColor::Green.light()))
}

pub fn adapter_active() -> Style {
    Style::from(ColorStyle::new(BaseColor::White, BaseColor::Green))
}

pub fn adapter_inactive() -> Style {
    Style::from(ColorStyle::new(BaseColor::White, BaseColor::Red))
}

pub fn register_changed() -> Style {
    Style::from(ColorStyle::new(BaseColor::Red, BaseColor::White))
}

pub fn trace_indicator() -> Style {
    Style::from(ColorStyle::new(BaseColor::Red, BaseColor::Blue)).combine(Effect::Blink)
}

pub fn trace_dropped() -> Style {
    Style::from(ColorStyle::new(BaseColor::Red, BaseColor::White)).combine(Effect::Bold)
}

pub fn field_offset() -> Style {
    Style::from(ColorStyle::new(BaseColor::Magenta, BaseColor::White)).combine(Effect::Bold)
}

pub fn field_value() -> Style {
    Style::from(ColorStyle::new(BaseColor::Cyan, BaseColor::White)).combine(Effect::Bold)
}

pub fn field_shortname() -> Style {
    Style::from(ColorStyle::new(BaseColor::Yellow, BaseColor::White)).combine(Effect::Bold)
}

pub fn drom_crc_ok() -> Style {
    Style::from(ColorStyle::new(BaseColor::Green, BaseColor::White)).combine(Effect::Bold)
}

pub fn drom_crc_bad() -> Style {
    Style::from(ColorStyle::new(BaseColor::Red, BaseColor::White)).combine(Effect::Bold)
}
