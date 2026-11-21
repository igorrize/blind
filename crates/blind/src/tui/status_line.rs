use blind_core::Usage;
use ratatui::{buffer::Buffer, layout::Rect, text::Line, widgets::Widget};

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

    fn text(&self) -> String {
        let parts = [
            Some(format!("{} {}", blind_core::NAME, blind_core::VERSION)),
            Some(self.model.to_owned()),
            self.usage.map(tokens),
            Some(self.hint.to_owned()),
        ];
        let shown: Vec<String> = parts.into_iter().flatten().collect();
        format!(" {}", shown.join(SEPARATOR))
    }
}

impl Widget for StatusLine<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, self.theme.status_line());
        Line::from(self.text()).render(area, buf);
    }
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
        StatusLine::new(
            "ollama-cloud/glm-5.3",
            usage,
            "Ctrl-C quit",
            Theme::default(),
        )
        .text()
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
}
