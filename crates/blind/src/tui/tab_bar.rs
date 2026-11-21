use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

use super::theme::Theme;

pub struct TabBar<'a> {
    titles: &'a [&'static str],
    active: usize,
    theme: Theme,
}

impl<'a> TabBar<'a> {
    pub fn new(titles: &'a [&'static str], active: usize, theme: Theme) -> Self {
        Self {
            titles,
            active,
            theme,
        }
    }

    fn label(&self, index: usize, title: &str) -> Span<'static> {
        if index == self.active {
            Span::styled(format!(" [ {title} ] "), self.theme.active_tab())
        } else {
            Span::styled(format!("   {title}   "), self.theme.inactive_tab())
        }
    }
}

impl Widget for TabBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let labels: Vec<Span> = self
            .titles
            .iter()
            .enumerate()
            .map(|(index, title)| self.label(index, title))
            .collect();
        Line::from(labels).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::rendered_text;

    #[test]
    fn only_the_active_tab_is_in_brackets() {
        let tabs = TabBar::new(&["root", "settings"], 1, Theme::default());
        let text = rendered_text(tabs, 40, 1);
        assert!(text.contains("[ settings ]"), "{text}");
        assert!(text.contains("root"), "{text}");
        assert!(!text.contains("[ root ]"), "{text}");
    }
}
