use ratatui::layout::{Constraint, Layout, Rect};

const SIDEBAR_WIDTH: u16 = 26;
const TAB_BAR_HEIGHT: u16 = 1;
const STATUS_LINE_HEIGHT: u16 = 1;

pub struct ScreenLayout {
    pub sidebar: Rect,
    pub tab_bar: Rect,
    pub content: Rect,
    pub status_line: Rect,
}

impl ScreenLayout {
    pub fn split(screen: Rect) -> Self {
        let [body, status_line] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(STATUS_LINE_HEIGHT)])
                .areas(screen);
        let [sidebar, main] =
            Layout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)]).areas(body);
        let [tab_bar, content] =
            Layout::vertical([Constraint::Length(TAB_BAR_HEIGHT), Constraint::Min(0)]).areas(main);
        Self {
            sidebar,
            tab_bar,
            content,
            status_line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_90x20() -> ScreenLayout {
        ScreenLayout::split(Rect::new(0, 0, 90, 20))
    }

    #[test]
    fn sidebar_keeps_its_width() {
        assert_eq!(screen_90x20().sidebar.width, SIDEBAR_WIDTH);
    }

    #[test]
    fn status_line_is_the_last_row() {
        let layout = screen_90x20();
        assert_eq!(layout.status_line.height, STATUS_LINE_HEIGHT);
        assert_eq!(layout.status_line.y, 19);
    }

    #[test]
    fn tab_bar_sits_right_of_sidebar_and_above_content() {
        let layout = screen_90x20();
        assert_eq!(layout.tab_bar.x, SIDEBAR_WIDTH);
        assert_eq!(layout.content.y, layout.tab_bar.y + TAB_BAR_HEIGHT);
    }
}
