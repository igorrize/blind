use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Borders, Paragraph, StatefulWidget, Widget},
};

use super::scroll::{Scroll, Scrolled};
use super::theme::Theme;
use super::wrap::{height_of, wrapped};
use crate::conversation::{Conversation, Entry, Speaker, Status};

const LABEL_WIDTH: usize = 6;
const INPUT_HEIGHT: u16 = 2;
const PROMPT_MARK: &str = "› ";
const CURSOR: &str = "█";
const HINT: &str = "Type a message and press Enter.";
const THINKING: &str = "root  thinking…";
const ERROR_LABEL: &str = "error";

pub struct ChatView<'a> {
    conversation: &'a Conversation,
    draft: &'a str,
    target: Option<&'a str>,
    theme: Theme,
}

impl<'a> ChatView<'a> {
    pub fn new(conversation: &'a Conversation, draft: &'a str, theme: Theme) -> Self {
        Self {
            conversation,
            draft,
            target: None,
            theme,
        }
    }

    pub fn sending_to(mut self, target: Option<&'a str>) -> Self {
        self.target = target;
        self
    }

    fn lines(&self, width: usize) -> Vec<Line<'static>> {
        if self.conversation.entries().is_empty() {
            return wrapped(HINT, width, self.theme.muted());
        }
        let spoken = self.conversation.entries().iter();
        let mut lines: Vec<Line> = spoken
            .flat_map(|entry| self.entry_lines(entry, width))
            .collect();
        lines.extend(self.status_lines(width));
        lines
    }

    fn entry_lines(&self, entry: &Entry, width: usize) -> Vec<Line<'static>> {
        let style = self.label_style(entry.speaker);
        let mut lines = labeled(label(entry.speaker), style, &entry.text, width);
        lines.push(Line::default());
        lines
    }

    fn status_lines(&self, width: usize) -> Vec<Line<'static>> {
        match self.conversation.status() {
            Status::Waiting => vec![Line::styled(THINKING, self.theme.muted())],
            Status::Failed(reason) => labeled(ERROR_LABEL, self.theme.error(), reason, width),
            Status::Idle | Status::Answering => Vec::new(),
        }
    }

    fn label_style(&self, speaker: Speaker) -> Style {
        match speaker {
            Speaker::Human => self.theme.human(),
            Speaker::Root => self.theme.root(),
            Speaker::Chain => self.theme.section_title(),
        }
    }

    fn target_lines(&self, width: u16) -> Vec<Line<'static>> {
        let target = self.target.unwrap_or_default();
        if target.is_empty() {
            return Vec::new();
        }
        wrapped(target, usize::from(width), self.theme.muted())
    }

    fn render_input(&self, target: Vec<Line<'static>>, area: Rect, buf: &mut Buffer) {
        let block = self.theme.bordered(Borders::TOP);
        let decoration = PROMPT_MARK.chars().count() + CURSOR.chars().count();
        let room = usize::from(block.inner(area).width).saturating_sub(decoration);
        let prompt = Line::raw(format!("{PROMPT_MARK}{}{CURSOR}", tail(self.draft, room)));
        let lines: Vec<Line> = target.into_iter().chain(std::iter::once(prompt)).collect();
        Paragraph::new(lines).block(block).render(area, buf);
    }
}

impl StatefulWidget for ChatView<'_> {
    type State = Scroll;

    fn render(self, area: Rect, buf: &mut Buffer, scroll: &mut Scroll) {
        let target = self.target_lines(area.width);
        let input_height = INPUT_HEIGHT.saturating_add(height_of(&target));
        let [transcript, input] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(input_height)]).areas(area);
        let lines = self.lines(usize::from(transcript.width));
        Scrolled::new(lines, self.theme).render(transcript, buf, scroll);
        self.render_input(target, input, buf);
    }
}

fn label(speaker: Speaker) -> &'static str {
    match speaker {
        Speaker::Human => "you",
        Speaker::Root => "root",
        Speaker::Chain => "chain",
    }
}

fn labeled(label: &'static str, style: Style, text: &str, width: usize) -> Vec<Line<'static>> {
    let room = width.saturating_sub(LABEL_WIDTH).max(1);
    let rows = textwrap::wrap(text, room);
    rows.into_iter()
        .enumerate()
        .map(|(index, row)| labeled_row(if index == 0 { label } else { "" }, style, &row))
        .collect()
}

fn labeled_row(label: &str, style: Style, row: &str) -> Line<'static> {
    let gutter = format!("{label:<LABEL_WIDTH$}");
    Line::from(vec![Span::styled(gutter, style), Span::raw(row.to_owned())])
}

fn tail(text: &str, count: usize) -> &str {
    let skipped = text.chars().count().saturating_sub(count);
    match text.char_indices().nth(skipped) {
        Some((start, _)) => &text[start..],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ReplyEvent;
    use crate::tui::test_support::{following, following_text, rendered_with, style_of};

    fn asked(question: &str) -> Conversation {
        let mut conversation = Conversation::default();
        conversation.say(question.to_owned());
        conversation
    }

    fn answered(question: &str, answer: &str) -> Conversation {
        let mut conversation = asked(question);
        conversation.receive(ReplyEvent::Text(answer.to_owned()));
        conversation
    }

    fn view(conversation: &Conversation, draft: &str, height: u16) -> String {
        let chat = ChatView::new(conversation, draft, Theme::default());
        following_text(chat, 40, height)
    }

    #[test]
    fn empty_conversation_shows_a_hint() {
        assert!(view(&Conversation::default(), "", 10).contains(HINT));
    }

    #[test]
    fn entries_are_labeled_by_speaker() {
        let text = view(&answered("hi", "Hello"), "", 10);
        assert!(text.contains("you   hi"), "{text}");
        assert!(text.contains("root  Hello"), "{text}");
    }

    #[test]
    fn human_and_root_labels_have_different_colors() {
        let conversation = answered("hi", "Hello");
        let theme = Theme::default();
        let buffer = following(ChatView::new(&conversation, "", theme), 40, 10);
        assert_eq!(style_of(&buffer, "you").fg, theme.human().fg);
        assert_eq!(style_of(&buffer, "root").fg, theme.root().fg);
    }

    #[test]
    fn chain_report_has_its_own_label_and_keeps_its_lines() {
        let mut conversation = Conversation::default();
        conversation.delegate("check".to_owned());
        conversation.report("explorer\n{}".to_owned());
        let text = view(&conversation, "", 10);
        assert!(text.contains("you   check"), "{text}");
        assert!(text.contains("chain explorer"), "{text}");
        assert!(!text.contains("thinking"), "{text}");
    }

    #[test]
    fn target_of_the_next_message_is_shown_above_the_input() {
        let conversation = Conversation::default();
        let chat = ChatView::new(&conversation, "go", Theme::default())
            .sending_to(Some("to chain: explorer"));
        let text = following_text(chat, 40, 10);
        let target = text.find("to chain: explorer").expect("target is shown");
        let input = text.find("› go").expect("input is shown");
        assert!(target < input, "{text}");
    }

    #[test]
    fn long_target_is_wrapped_and_the_input_stays_visible() {
        let conversation = Conversation::default();
        let target = "to chain: explorer → reviewer · Tab: talk to root";
        let chat = ChatView::new(&conversation, "go", Theme::default()).sending_to(Some(target));
        let text = following_text(chat, 30, 10);
        assert!(text.contains("talk to root"), "{text}");
        assert!(text.contains("› go"), "{text}");
    }

    #[test]
    fn scrolled_chat_shows_earlier_entries_and_how_much_is_below() {
        let mut conversation = Conversation::default();
        for number in 1..=12 {
            conversation.delegate(format!("message {number}"));
        }
        let mut scroll = Scroll::default();
        let theme = Theme::default();
        rendered_with(ChatView::new(&conversation, "", theme), 40, 10, &mut scroll);
        scroll.up(6);
        let buffer = rendered_with(ChatView::new(&conversation, "", theme), 40, 10, &mut scroll);
        let text = crate::tui::test_support::buffer_text(&buffer);
        assert!(text.contains("more · End"), "{text}");
        assert!(!text.contains("message 12"), "{text}");
        assert!(text.contains("message 9"), "{text}");
    }

    #[test]
    fn waiting_for_root_is_visible() {
        assert!(view(&asked("hi"), "", 10).contains(THINKING));
    }

    #[test]
    fn failure_is_shown_as_an_error() {
        let mut conversation = asked("hi");
        conversation.receive(ReplyEvent::Failed("provider answered 401".to_owned()));
        let text = view(&conversation, "", 10);
        assert!(text.contains("error provider answered 401"), "{text}");
    }

    #[test]
    fn long_text_wraps_under_its_label() {
        let lines = labeled(
            "root",
            Style::default(),
            "one two three four five six seven",
            20,
        );
        let rows: Vec<String> = lines.iter().map(Line::to_string).collect();
        assert_eq!(
            rows,
            ["root  one two three", "      four five six", "      seven"]
        );
    }

    #[test]
    fn newest_lines_stay_visible_when_the_transcript_overflows() {
        let mut conversation = answered("first question", "first answer");
        conversation.receive(ReplyEvent::Done(blind_core::Usage::default()));
        conversation.say("second question".to_owned());
        conversation.receive(ReplyEvent::Text("second answer".to_owned()));
        let text = view(&conversation, "", 7);
        assert!(text.contains("second answer"), "{text}");
        assert!(!text.contains("first question"), "{text}");
    }

    #[test]
    fn input_shows_the_draft_with_a_cursor() {
        assert!(view(&Conversation::default(), "hello", 10).contains("› hello█"));
    }

    #[test]
    fn tail_keeps_the_last_characters() {
        assert_eq!(tail("привет", 3), "вет");
        assert_eq!(tail("hi", 10), "hi");
        assert_eq!(tail("hi", 0), "");
    }
}
