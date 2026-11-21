use catppuccin::{FlavorColors, PALETTE};
use ratatui::{
    style::{Color, Modifier, Style},
    widgets::{Block, Borders},
};

use crate::ui_config::Flavor;

#[derive(Clone, Copy)]
pub struct Theme {
    base: Color,
    mantle: Color,
    surface: Color,
    overlay: Color,
    subtext: Color,
    text: Color,
    mauve: Color,
    blue: Color,
    green: Color,
    red: Color,
}

impl Theme {
    pub fn of(flavor: Flavor) -> Self {
        let colors = flavor_colors(flavor);
        Self {
            base: rgb(colors.base),
            mantle: rgb(colors.mantle),
            surface: rgb(colors.surface1),
            overlay: rgb(colors.overlay0),
            subtext: rgb(colors.subtext0),
            text: rgb(colors.text),
            mauve: rgb(colors.mauve),
            blue: rgb(colors.blue),
            green: rgb(colors.green),
            red: rgb(colors.red),
        }
    }

    pub fn screen(self) -> Style {
        Style::default().fg(self.text).bg(self.base)
    }

    pub fn sidebar(self) -> Style {
        Style::default().fg(self.subtext).bg(self.mantle)
    }

    pub fn status_line(self) -> Style {
        Style::default().fg(self.subtext).bg(self.mantle)
    }

    pub fn section_title(self) -> Style {
        bold(self.mauve)
    }

    pub fn open_item(self) -> Style {
        bold(self.text)
    }

    pub fn cursor(self) -> Style {
        bold(self.text).bg(self.surface)
    }

    pub fn active_tab(self) -> Style {
        bold(self.base).bg(self.blue)
    }

    pub fn inactive_tab(self) -> Style {
        Style::default().fg(self.subtext)
    }

    pub fn muted(self) -> Style {
        Style::default().fg(self.overlay)
    }

    pub fn human(self) -> Style {
        bold(self.blue)
    }

    pub fn root(self) -> Style {
        bold(self.green)
    }

    pub fn error(self) -> Style {
        bold(self.red)
    }

    pub fn bordered(self, borders: Borders) -> Block<'static> {
        Block::default()
            .borders(borders)
            .border_style(Style::default().fg(self.surface))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::of(Flavor::default())
    }
}

fn flavor_colors(flavor: Flavor) -> FlavorColors {
    match flavor {
        Flavor::Latte => PALETTE.latte.colors,
        Flavor::Frappe => PALETTE.frappe.colors,
        Flavor::Macchiato => PALETTE.macchiato.colors,
        Flavor::Mocha => PALETTE.mocha.colors,
    }
}

fn rgb(color: catppuccin::Color) -> Color {
    Color::Rgb(color.rgb.r, color.rgb.g, color.rgb.b)
}

fn bold(color: Color) -> Style {
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mocha_uses_the_official_base_and_text_colors() {
        let screen = Theme::of(Flavor::Mocha).screen();
        assert_eq!(screen.bg, Some(Color::Rgb(0x1e, 0x1e, 0x2e)));
        assert_eq!(screen.fg, Some(Color::Rgb(0xcd, 0xd6, 0xf4)));
    }

    #[test]
    fn latte_is_light_and_mocha_is_dark() {
        let latte = Theme::of(Flavor::Latte).screen();
        assert_eq!(latte.bg, Some(Color::Rgb(0xef, 0xf1, 0xf5)));
        assert_ne!(latte.bg, Theme::of(Flavor::Mocha).screen().bg);
    }

    #[test]
    fn planned_items_are_visibly_dimmer_than_text() {
        let theme = Theme::default();
        assert_ne!(theme.muted().fg, theme.sidebar().fg);
        assert_ne!(theme.muted().fg, theme.sidebar().bg);
    }
}
