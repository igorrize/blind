use ratatui::{buffer::Buffer, layout::Rect, style::Style, text::Line, widgets::StatefulWidget};

use super::scroll::{Scroll, Scrolled};
use super::theme::Theme;
use super::wrap::wrapped;
use crate::chain_log::{ChainLog, Kind, LogLine};

const HINT: &str =
    "No chain has run yet. Ctrl-X c builds a chain, then your next message goes to it.";

pub struct ChainView<'a> {
    log: &'a ChainLog,
    theme: Theme,
}

impl<'a> ChainView<'a> {
    pub fn new(log: &'a ChainLog, theme: Theme) -> Self {
        Self { log, theme }
    }

    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        if self.log.lines().is_empty() {
            return wrapped(HINT, width, self.theme.muted());
        }
        self.log
            .lines()
            .iter()
            .flat_map(|line| self.rows(line, width))
            .collect()
    }

    fn rows(&self, line: &LogLine, width: usize) -> Vec<Line<'static>> {
        let style = self.style(line.kind);
        let rows = textwrap::wrap(&line.text, width.max(1));
        let before = starts_a_block(line.kind).then(Line::default);
        let wrapped = rows
            .into_iter()
            .map(move |row| Line::styled(row.into_owned(), style));
        before.into_iter().chain(wrapped).collect()
    }

    fn style(&self, kind: Kind) -> Style {
        match kind {
            Kind::Run => self.theme.section_title(),
            Kind::Step => self.theme.root(),
            Kind::Said | Kind::Result => Style::default(),
            Kind::Called | Kind::Note | Kind::Summary => self.theme.muted(),
            Kind::Refused | Kind::Problem => self.theme.error(),
        }
    }
}

fn starts_a_block(kind: Kind) -> bool {
    matches!(kind, Kind::Run | Kind::Step)
}

impl StatefulWidget for ChainView<'_> {
    type State = Scroll;

    fn render(self, area: Rect, buf: &mut Buffer, scroll: &mut Scroll) {
        let lines = self.lines(usize::from(area.width));
        Scrolled::new(lines, self.theme).render(area, buf, scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain_log::{Kind as LineKind, RunEvent};
    use crate::lineup::Lineup;
    use crate::tui::test_support::{following, following_text, style_of};

    fn by_explorer(kind: LineKind, text: &str) -> RunEvent {
        RunEvent::Step {
            agent: "explorer".to_owned(),
            kind,
            text: text.to_owned(),
        }
    }

    fn log_of_a_run() -> ChainLog {
        let mut log = ChainLog::default();
        log.start(&Lineup::parse(&["explorer".to_owned()]), "check it");
        log.receive(RunEvent::StepStarted(vec!["explorer".to_owned()]));
        log.receive(by_explorer(LineKind::Called, "· grep {}"));
        log.receive(by_explorer(LineKind::Refused, "✗ read_file {} — not given"));
        log.receive(RunEvent::StepResult("{\n  \"files\": []\n}".to_owned()));
        log
    }

    #[test]
    fn empty_log_tells_how_to_build_a_chain() {
        let log = ChainLog::default();
        let text = following_text(ChainView::new(&log, Theme::default()), 100, 5);
        assert!(text.contains("Ctrl-X c builds a chain"), "{text}");
    }

    #[test]
    fn hint_is_wrapped_on_a_narrow_screen() {
        let log = ChainLog::default();
        let text = following_text(ChainView::new(&log, Theme::default()), 40, 5);
        assert!(text.contains("to it."), "{text}");
    }

    #[test]
    fn run_is_shown_with_its_steps_calls_and_result() {
        let log = log_of_a_run();
        let text = following_text(ChainView::new(&log, Theme::default()), 60, 14);
        for expected in [
            "explorer · check it",
            "→ explorer",
            "· grep {}",
            "✗ read_file {} — not given",
            "\"files\": []",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text}");
        }
    }

    #[test]
    fn refused_call_is_shown_as_an_error_and_a_done_call_is_muted() {
        let theme = Theme::default();
        let log = log_of_a_run();
        let buffer = following(ChainView::new(&log, theme), 60, 14);
        assert_eq!(style_of(&buffer, "✗ read_file").fg, theme.error().fg);
        assert_eq!(style_of(&buffer, "· grep").fg, theme.muted().fg);
    }

    #[test]
    fn long_log_shows_its_end() {
        let mut log = log_of_a_run();
        for number in 0..30 {
            log.receive(by_explorer(LineKind::Note, &format!("note {number}")));
        }
        let text = following_text(ChainView::new(&log, Theme::default()), 60, 6);
        assert!(text.contains("note 29"), "{text}");
        assert!(!text.contains("→ explorer"), "{text}");
    }
}
