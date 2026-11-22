use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Borders, Paragraph, Widget},
};

use super::app::Tab;
use super::theme::Theme;

const POINTER: &str = "› ";
const NO_POINTER: &str = "  ";

const SECTIONS: [Section; 4] = [
    Section {
        title: "AGENTS",
        items: &[Item::Opens(Tab::Root)],
    },
    Section {
        title: "WINDOWS",
        items: &[
            Item::Planned("approvals"),
            Item::Planned("audit"),
            Item::Planned("logs"),
            Item::Planned("contracts"),
            Item::Opens(Tab::Settings),
        ],
    },
    Section {
        title: "RUNS",
        items: &[Item::Opens(Tab::Chain)],
    },
    Section {
        title: "SESSIONS",
        items: &[],
    },
];

pub struct Sidebar {
    active: Tab,
    cursor: Option<Tab>,
    theme: Theme,
}

struct Section {
    title: &'static str,
    items: &'static [Item],
}

enum Item {
    Opens(Tab),
    Planned(&'static str),
}

impl Sidebar {
    pub fn new(active: Tab, cursor: Option<Tab>, theme: Theme) -> Self {
        Self {
            active,
            cursor,
            theme,
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        SECTIONS
            .iter()
            .flat_map(|section| self.section_lines(section))
            .collect()
    }

    fn section_lines(&self, section: &Section) -> Vec<Line<'static>> {
        let heading = Line::styled(section.title, self.theme.section_title());
        let items = section.items.iter().map(|item| self.item_line(item));
        std::iter::once(heading)
            .chain(items)
            .chain(std::iter::once(Line::default()))
            .collect()
    }

    fn item_line(&self, item: &Item) -> Line<'static> {
        match item {
            Item::Planned(title) => {
                Line::styled(format!("{NO_POINTER}{title}"), self.theme.muted())
            }
            Item::Opens(tab) => self.tab_line(*tab),
        }
    }

    fn tab_line(&self, tab: Tab) -> Line<'static> {
        let under_cursor = self.cursor == Some(tab);
        let pointer = if under_cursor { POINTER } else { NO_POINTER };
        Line::styled(format!("{pointer}{}", tab.title()), self.tab_style(tab))
    }

    fn tab_style(&self, tab: Tab) -> Style {
        if self.cursor == Some(tab) {
            self.theme.cursor()
        } else if self.active == tab {
            self.theme.open_item()
        } else {
            Style::default()
        }
    }
}

impl Widget for Sidebar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(self.lines())
            .block(self.theme.bordered(Borders::RIGHT))
            .style(self.theme.sidebar())
            .render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{rendered, rendered_text, style_of};

    fn sidebar(cursor: Option<Tab>) -> Sidebar {
        Sidebar::new(Tab::Root, cursor, Theme::default())
    }

    fn text(cursor: Option<Tab>) -> String {
        rendered_text(sidebar(cursor), 26, 14)
    }

    #[test]
    fn shows_every_section_with_its_items() {
        let text = text(None);
        for expected in [
            "AGENTS",
            "root",
            "WINDOWS",
            "approvals",
            "settings",
            "RUNS",
            "chain",
            "SESSIONS",
        ] {
            assert!(text.contains(expected), "missing {expected:?}");
        }
    }

    #[test]
    fn unfocused_sidebar_has_no_pointer() {
        assert!(!text(None).contains(POINTER));
    }

    #[test]
    fn pointer_marks_the_item_under_the_cursor() {
        let text = text(Some(Tab::Settings));
        assert!(text.contains("› settings"), "{text}");
        assert!(!text.contains("› root"), "{text}");
    }

    #[test]
    fn planned_items_use_the_muted_color_and_openable_ones_do_not() {
        let theme = Theme::default();
        let buffer = rendered(sidebar(None), 26, 14);
        assert_eq!(style_of(&buffer, "approvals").fg, theme.muted().fg);
        assert_ne!(style_of(&buffer, "settings").fg, theme.muted().fg);
    }
}
