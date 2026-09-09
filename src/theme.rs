//! The whole palette. Dark unless GROVE_THEME=light, which only flips the
//! syntect grammar theme and the two greys the chrome uses.

use ratatui::style::Color;

pub fn is_light() -> bool {
    std::env::var("GROVE_THEME").map(|v| v.eq_ignore_ascii_case("light")).unwrap_or(false)
}

pub fn selection_bg() -> Color {
    if is_light() { Color::Rgb(0xd6, 0xe4, 0xff) } else { Color::Rgb(0x09, 0x47, 0x71) }
}

pub fn dim() -> Color {
    if is_light() { Color::Rgb(0x6a, 0x6f, 0x78) } else { Color::Rgb(0x85, 0x8b, 0x98) }
}

pub fn chrome() -> Color {
    if is_light() { Color::Rgb(0x33, 0x37, 0x3d) } else { Color::Rgb(0xcc, 0xcc, 0xcc) }
}

pub fn rule() -> Color {
    if is_light() { Color::Rgb(0xc8, 0xcc, 0xd2) } else { Color::Rgb(0x3a, 0x3f, 0x4b) }
}
