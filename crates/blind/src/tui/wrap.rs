use ratatui::{style::Style, text::Line};
use textwrap::{Options, core::display_width};

pub fn wrapped(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    hanging(text, width, "", style)
}

pub fn hanging(text: &str, width: usize, indent: &str, style: Style) -> Vec<Line<'static>> {
    let width = width.max(1);
    let leaves_room = display_width(indent) * 2 < width;
    let indent = if leaves_room { indent } else { "" };
    let options = Options::new(width).subsequent_indent(indent);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|row| Line::styled(row.into_owned(), style))
        .collect()
}

pub fn height_of(lines: &[Line<'_>]) -> u16 {
    u16::try_from(lines.len()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn short_text_stays_on_one_line() {
        let lines = wrapped("no chain yet", 40, Style::default());
        assert_eq!(rows(&lines), ["no chain yet"]);
    }

    #[test]
    fn long_text_is_split_between_words_and_no_row_is_wider_than_the_room() {
        let text =
            "No chain has run yet. Ctrl-X c builds a chain, then your next message goes to it.";
        let lines = wrapped(text, 20, Style::default());
        assert!(lines.len() > 1);
        assert!(
            lines.iter().all(|line| line.width() <= 20),
            "{:?}",
            rows(&lines)
        );
        assert_eq!(rows(&lines).join(" "), text);
    }

    #[test]
    fn new_lines_in_the_text_are_kept() {
        let lines = wrapped("explorer\n{}", 40, Style::default());
        assert_eq!(rows(&lines), ["explorer", "{}"]);
    }

    #[test]
    fn continuation_rows_are_indented() {
        let text = "reviewer cannot go first: needs files";
        let lines = rows(&hanging(text, 20, "   ", Style::default()));
        assert!(lines.len() > 1, "{lines:?}");
        assert!(!lines[0].starts_with(' '), "{lines:?}");
        assert!(
            lines[1..].iter().all(|row| row.starts_with("   ")),
            "{lines:?}"
        );
        assert!(
            lines.iter().all(|row| row.chars().count() <= 20),
            "{lines:?}"
        );
        let words: Vec<&str> = lines
            .iter()
            .flat_map(|row| row.split_whitespace())
            .collect();
        assert_eq!(words.join(" "), text);
    }

    #[test]
    fn indent_is_dropped_when_it_would_leave_no_room_for_words() {
        let lines = hanging("reviewer needs files", 8, "      ", Style::default());
        assert!(lines.iter().all(|line| !line.to_string().starts_with(' ')));
    }

    #[test]
    fn zero_width_does_not_panic() {
        assert!(!wrapped("text", 0, Style::default()).is_empty());
    }

    #[test]
    fn style_is_kept_on_every_row() {
        let style = Style::default().fg(ratatui::style::Color::Red);
        let lines = wrapped("one two three four", 5, style);
        assert!(lines.iter().all(|line| line.style == style));
    }
}
