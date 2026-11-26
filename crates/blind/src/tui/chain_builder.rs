use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Flex, Layout, Rect},
    style::Style,
    text::Line,
    widgets::{Borders, Clear, Paragraph, Widget},
};

use super::theme::Theme;
use super::wrap::{hanging, height_of, wrapped};
use crate::crew::Candidate;
use crate::lineup::Lineup;

const TITLE: &str = " chain ";
const NEXT_STEP: &str = "?";
const NO_AGENTS: &str = "No agents: put agent folders into .blind/agents and restart blind.";
const NAME_WIDTH: usize = 14;
const UNDER_DESCRIPTION: &str = "                   ";
const WIDEST: u16 = 96;
const FRAME: u16 = 2;

pub struct ChainBuilder<'a> {
    lineup: &'a Lineup,
    beside: bool,
    candidates: &'a [Candidate],
    notice: Option<&'a str>,
    theme: Theme,
}

impl<'a> ChainBuilder<'a> {
    pub fn new(
        lineup: &'a Lineup,
        beside: bool,
        candidates: &'a [Candidate],
        theme: Theme,
    ) -> Self {
        Self {
            lineup,
            beside,
            candidates,
            notice: None,
            theme,
        }
    }

    pub fn with_notice(mut self, notice: Option<&'a str>) -> Self {
        self.notice = notice;
        self
    }

    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        let so_far = self.lineup.with(NEXT_STEP, self.beside).to_string();
        let chain = wrapped(&so_far, width, self.theme.open_item());
        let candidates = self.candidate_lines(width);
        let notice = self.notice_lines(width);
        chain
            .into_iter()
            .chain(std::iter::once(Line::default()))
            .chain(candidates)
            .chain(notice)
            .collect()
    }

    fn notice_lines(&self, width: usize) -> Vec<Line<'static>> {
        let Some(notice) = self.notice else {
            return Vec::new();
        };
        let mut lines = vec![Line::default()];
        lines.extend(wrapped(notice, width, self.theme.error()));
        lines
    }

    fn candidate_lines(&self, width: usize) -> Vec<Line<'static>> {
        if self.candidates.is_empty() {
            return wrapped(NO_AGENTS, width, self.theme.muted());
        }
        let numbered = self.candidates.iter().enumerate();
        numbered
            .flat_map(|(index, candidate)| self.candidate_rows(index + 1, candidate, width))
            .collect()
    }

    fn candidate_rows(
        &self,
        number: usize,
        candidate: &Candidate,
        width: usize,
    ) -> Vec<Line<'static>> {
        let name = &candidate.name;
        let (text, style) = match &candidate.obstacle {
            None => (
                format!(" {number}  {name:<NAME_WIDTH$} {}", candidate.description),
                Style::default(),
            ),
            Some(obstacle) => (
                format!("    {name:<NAME_WIDTH$} {obstacle}"),
                self.theme.muted(),
            ),
        };
        hanging(&text, width, UNDER_DESCRIPTION, style)
    }
}

impl Widget for ChainBuilder<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = WIDEST.min(area.width);
        let lines = self.lines(usize::from(width.saturating_sub(FRAME)));
        let window = centered(area, width, height_of(&lines).saturating_add(FRAME));
        let block = self.theme.bordered(Borders::ALL).title(TITLE);
        Clear.render(window, buf);
        Paragraph::new(lines)
            .block(block)
            .style(self.theme.sidebar())
            .render(window, buf);
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [row] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [window] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(row);
    window
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{buffer_text, rendered, rendered_text, style_of};

    fn candidate(name: &str, obstacle: Option<&str>) -> Candidate {
        Candidate {
            name: name.to_owned(),
            description: format!("The {name}."),
            obstacle: obstacle.map(str::to_owned),
        }
    }

    fn explorer_and_reviewer() -> Vec<Candidate> {
        vec![
            candidate("explorer", None),
            candidate("reviewer", Some("cannot go first: needs files")),
        ]
    }

    fn lineup(arguments: &[&str]) -> Lineup {
        let arguments: Vec<String> = arguments.iter().map(|text| (*text).to_owned()).collect();
        Lineup::parse(&arguments)
    }

    fn shown(lineup: &Lineup, beside: bool) -> String {
        let candidates = explorer_and_reviewer();
        let builder = ChainBuilder::new(lineup, beside, &candidates, Theme::default());
        rendered_text(builder, 80, 10)
    }

    #[test]
    fn empty_chain_asks_for_the_first_step() {
        let text = shown(&Lineup::default(), false);
        assert!(text.contains(" chain "), "{text}");
        assert!(text.contains("│?"), "{text}");
    }

    #[test]
    fn chain_so_far_ends_with_the_place_for_the_next_step() {
        let text = shown(&lineup(&["explorer", "reviewer"]), false);
        assert!(text.contains("explorer → reviewer → ?"), "{text}");
    }

    #[test]
    fn place_beside_the_last_step_is_shown_with_a_plus() {
        let text = shown(&lineup(&["explorer", "reviewer"]), true);
        assert!(text.contains("explorer → reviewer + ?"), "{text}");
    }

    #[test]
    fn fitting_candidate_has_a_number_and_an_unfit_one_has_the_reason() {
        let theme = Theme::default();
        let candidates = explorer_and_reviewer();
        let empty = Lineup::default();
        let buffer = rendered(ChainBuilder::new(&empty, false, &candidates, theme), 80, 10);
        let text = buffer_text(&buffer);
        assert!(text.contains(" 1  explorer       The explorer."), "{text}");
        assert!(
            text.contains("    reviewer       cannot go first: needs files"),
            "{text}"
        );
        assert_eq!(style_of(&buffer, "reviewer").fg, theme.muted().fg);
        assert_ne!(style_of(&buffer, "explorer").fg, theme.muted().fg);
    }

    #[test]
    fn long_reason_is_wrapped_inside_the_window_on_a_narrow_screen() {
        let candidates = vec![candidate(
            "reviewer",
            Some("does not fit after planner: `files` is required but not guaranteed"),
        )];
        let empty = Lineup::default();
        let builder = ChainBuilder::new(&empty, false, &candidates, Theme::default());
        let text = rendered_text(builder, 44, 12);
        assert!(text.contains("guaranteed"), "{text}");
    }

    #[test]
    fn notice_is_shown_below_the_candidates() {
        let candidates = explorer_and_reviewer();
        let empty = Lineup::default();
        let builder = ChainBuilder::new(&empty, false, &candidates, Theme::default())
            .with_notice(Some("root cannot take the result of `explorer`"));
        let text = rendered_text(builder, 80, 12);
        let notice = text.find("root cannot take").expect("notice is shown");
        let candidate = text
            .find("explorer       The explorer.")
            .expect("candidate is shown");
        assert!(candidate < notice, "{text}");
    }

    #[test]
    fn crew_without_agents_says_where_to_put_them() {
        let empty = Lineup::default();
        let builder = ChainBuilder::new(&empty, false, &[], Theme::default());
        let text = rendered_text(builder, 80, 10);
        assert!(text.contains(".blind/agents"), "{text}");
    }

    #[test]
    fn window_fits_into_a_small_screen() {
        let text = {
            let candidates = explorer_and_reviewer();
            let empty = Lineup::default();
            let builder = ChainBuilder::new(&empty, false, &candidates, Theme::default());
            rendered_text(builder, 30, 3)
        };
        assert!(text.contains(" chain "), "{text}");
    }
}
