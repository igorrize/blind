use blind_core::Usage;
use ratatui::{buffer::Buffer, layout::Rect, text::Line, widgets::Widget};
use textwrap::core::display_width;

use super::theme::Theme;

const SEPARATOR: &str = " · ";

pub struct StatusLine<'a> {
    model: &'a str,
    usage: Option<Usage>,
    hint: &'a str,
    theme: Theme,
}

impl<'a> StatusLine<'a> {
    pub fn new(model: &'a str, usage: Option<Usage>, hint: &'a str, theme: Theme) -> Self {
        Self {
            model,
            usage,
            hint,
            theme,
        }
    }

    fn text(&self, width: usize) -> String {
        let version = format!("{} {}", blind_core::NAME, blind_core::VERSION);
        let model = Some(self.model.to_owned());
        let tokens = self.usage.map(tokens);
        let hint = Some(self.hint.to_owned());
        let from_fullest = [
            joined([Some(version), model.clone(), tokens.clone(), hint.clone()]),
            joined([None, model.clone(), tokens, hint.clone()]),
            joined([None, model, None, hint.clone()]),
            joined([None, None, None, hint]),
        ];
        let fitting = from_fullest
            .iter()
            .find(|text| display_width(text) <= width);
        fitting.unwrap_or(&from_fullest[3]).clone()
    }
}

impl Widget for StatusLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, self.theme.status_line());
        Line::from(self.text(usize::from(area.width))).render(area, buf);
    }
}

fn joined(parts: [Option<String>; 4]) -> String {
    let shown: Vec<String> = parts.into_iter().flatten().collect();
    format!(" {}", shown.join(SEPARATOR))
}

fn tokens(usage: Usage) -> String {
    format!(
        "tokens: {} in, {} out",
        usage.input_tokens, usage.output_tokens
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(usage: Option<Usage>) -> String {
        status_in(usage, 200)
    }

    fn status_in(usage: Option<Usage>, width: usize) -> String {
        StatusLine::new(
            "ollama-cloud/glm-5.3",
            usage,
            "Ctrl-C quit",
            Theme::default(),
        )
        .text(width)
    }

    #[test]
    fn shows_version_model_and_hint() {
        assert_eq!(
            status(None),
            format!(
                " blind {} · ollama-cloud/glm-5.3 · Ctrl-C quit",
                blind_core::VERSION
            )
        );
    }

    #[test]
    fn shows_tokens_of_the_last_answer() {
        let usage = Usage {
            input_tokens: 19,
            output_tokens: 247,
        };
        let text = status(Some(usage));
        assert!(text.contains(" · tokens: 19 in, 247 out · "), "{text}");
    }

    #[test]
    fn narrow_screen_drops_the_version_tokens_and_model_but_keeps_the_hint() {
        let usage = Some(Usage {
            input_tokens: 19,
            output_tokens: 247,
        });
        let without_version = status_in(usage, 70);
        assert!(!without_version.contains("blind "), "{without_version}");
        assert!(without_version.contains("tokens"), "{without_version}");
        assert_eq!(status_in(usage, 40), " ollama-cloud/glm-5.3 · Ctrl-C quit");
        assert_eq!(status_in(usage, 20), " Ctrl-C quit");
        assert_eq!(status_in(usage, 5), " Ctrl-C quit");
    }
}
