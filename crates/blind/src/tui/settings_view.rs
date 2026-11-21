use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Borders, Paragraph, Widget},
};

use super::theme::Theme;
use crate::model_catalog::Catalog;
use crate::ui_config::Flavor;

const HELP: &str = "↑↓ move · Enter select · r reload models";
const LOADING: &str = "loading models…";
const POINTER: &str = "› ";
const NO_POINTER: &str = "  ";
const CURRENT_MARK: &str = "  ● current";
const NOTICE_HEIGHT: u16 = 1;

pub struct SettingsView<'a> {
    pub theme: Theme,
    pub flavor: Flavor,
    pub catalog: &'a Catalog,
    pub current_model: &'a str,
    pub row: usize,
    pub notice: Option<&'a str>,
}

struct Listing {
    lines: Vec<Line<'static>>,
    cursor_line: usize,
}

impl SettingsView<'_> {
    fn listing(&self) -> Listing {
        let mut lines = vec![Line::styled("Theme", self.theme.section_title())];
        lines.extend(self.flavor_rows());
        lines.push(Line::default());
        lines.push(Line::styled("Root model", self.theme.section_title()));
        let first_model_line = lines.len();
        lines.extend(self.catalog_lines());
        Listing {
            lines,
            cursor_line: self.cursor_line(first_model_line),
        }
    }

    fn cursor_line(&self, first_model_line: usize) -> usize {
        let first_flavor_line = 1;
        match self.row.checked_sub(Flavor::ALL.len()) {
            Some(model_row) => first_model_line + model_row,
            None => first_flavor_line + self.row,
        }
    }

    fn flavor_rows(&self) -> Vec<Line<'static>> {
        Flavor::ALL
            .iter()
            .map(|flavor| self.choice(flavor.name(), flavor.index(), *flavor == self.flavor))
            .collect()
    }

    fn catalog_lines(&self) -> Vec<Line<'static>> {
        match self.catalog {
            Catalog::NotLoaded | Catalog::Loading => {
                vec![Line::styled(LOADING, self.theme.muted())]
            }
            Catalog::Failed(reason) => {
                vec![Line::styled(format!("error  {reason}"), self.theme.error())]
            }
            Catalog::Loaded(models) => self.model_rows(models),
        }
    }

    fn model_rows(&self, models: &[String]) -> Vec<Line<'static>> {
        models
            .iter()
            .enumerate()
            .map(|(index, model)| {
                let row = Flavor::ALL.len() + index;
                self.choice(model, row, model == self.current_model)
            })
            .collect()
    }

    fn choice(&self, label: &str, row: usize, current: bool) -> Line<'static> {
        let highlighted = row == self.row;
        let pointer = if highlighted { POINTER } else { NO_POINTER };
        let mark = if current { CURRENT_MARK } else { "" };
        let style = if highlighted {
            self.theme.cursor()
        } else {
            Style::default()
        };
        Line::styled(format!("{pointer}{label}{mark}"), style)
    }

    fn notice_line(&self) -> Line<'static> {
        Line::styled(self.notice.unwrap_or(HELP).to_owned(), self.theme.muted())
    }
}

impl Widget for SettingsView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = self.theme.bordered(Borders::TOP);
        let [list, notice] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(NOTICE_HEIGHT)])
                .areas(block.inner(area));
        let listing = self.listing();
        let hidden = first_visible(listing.cursor_line, usize::from(list.height));
        let visible: Vec<Line> = listing.lines.into_iter().skip(hidden).collect();
        self.notice_line().render(notice, buf);
        block.render(area, buf);
        Paragraph::new(visible).render(list, buf);
    }
}

fn first_visible(cursor_line: usize, room: usize) -> usize {
    (cursor_line + 1).saturating_sub(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::rendered_text;

    fn loaded(models: &[&str]) -> Catalog {
        Catalog::Loaded(models.iter().map(|model| (*model).to_owned()).collect())
    }

    fn view(catalog: &Catalog, row: usize, notice: Option<&str>, height: u16) -> String {
        let settings = SettingsView {
            theme: Theme::default(),
            flavor: Flavor::Mocha,
            catalog,
            current_model: "glm-5.3",
            row,
            notice,
        };
        rendered_text(settings, 50, height)
    }

    #[test]
    fn lists_every_flavor_and_marks_the_current_one() {
        let text = view(&Catalog::default(), Flavor::Mocha.index(), None, 16);
        for name in ["latte", "frappe", "macchiato"] {
            assert!(text.contains(name), "missing {name:?}");
        }
        assert!(text.contains("› mocha  ● current"), "{text}");
    }

    #[test]
    fn shows_loading_until_models_arrive() {
        let text = view(&Catalog::default(), 0, None, 16);
        assert!(text.contains("Root model"), "{text}");
        assert!(text.contains(LOADING), "{text}");
        assert!(text.contains(HELP), "{text}");
    }

    #[test]
    fn rows_after_the_flavors_point_at_models() {
        let catalog = loaded(&["beam", "glm-5.3", "kimi"]);
        let text = view(&catalog, Flavor::ALL.len() + 1, None, 16);
        assert!(text.contains("› glm-5.3  ● current"), "{text}");
        assert!(text.contains("  beam"), "{text}");
        assert!(!text.contains("› mocha"), "{text}");
    }

    #[test]
    fn notice_replaces_the_help_line() {
        let text = view(&loaded(&["glm-5.3"]), 0, Some("theme is now latte"), 16);
        assert!(text.contains("theme is now latte"), "{text}");
        assert!(!text.contains(HELP), "{text}");
    }

    #[test]
    fn failure_is_shown_instead_of_the_models() {
        let catalog = Catalog::Failed("provider answered 401".to_owned());
        let text = view(&catalog, 0, None, 16);
        assert!(text.contains("error  provider answered 401"), "{text}");
    }

    #[test]
    fn list_scrolls_to_keep_the_highlighted_row_visible() {
        let catalog = loaded(&["a1", "b2", "c3", "d4", "e5", "f6"]);
        let text = view(&catalog, Flavor::ALL.len() + 5, None, 8);
        assert!(text.contains("› f6"), "{text}");
        assert!(!text.contains("latte"), "{text}");
        assert!(text.contains(HELP), "{text}");
    }

    #[test]
    fn first_visible_line_follows_the_cursor() {
        assert_eq!(first_visible(0, 3), 0);
        assert_eq!(first_visible(2, 3), 0);
        assert_eq!(first_visible(5, 3), 3);
    }
}
