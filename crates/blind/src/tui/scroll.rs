use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Borders, Paragraph, StatefulWidget, Widget},
};

use super::theme::Theme;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scroll {
    pinned_top: Option<usize>,
    last_top: usize,
    page: usize,
}

pub struct Scrolled {
    lines: Vec<Line<'static>>,
    theme: Theme,
}

impl Scroll {
    pub fn up(&mut self, rows: usize) {
        self.pinned_top = Some(self.top().saturating_sub(rows));
    }

    pub fn down(&mut self, rows: usize) {
        let wanted = self.top() + rows;
        self.pinned_top = (wanted < self.last_top).then_some(wanted);
    }

    pub fn page_up(&mut self) {
        self.up(self.page);
    }

    pub fn page_down(&mut self) {
        self.down(self.page);
    }

    pub fn rewind(&mut self) {
        self.pinned_top = Some(0);
    }

    pub fn follow(&mut self) {
        self.pinned_top = None;
    }

    #[cfg(test)]
    pub fn is_following(&self) -> bool {
        self.pinned_top.is_none()
    }

    fn top(&self) -> usize {
        self.pinned_top.unwrap_or(self.last_top).min(self.last_top)
    }

    fn show<T>(&mut self, lines: Vec<T>, height: usize) -> (Vec<T>, usize) {
        self.last_top = lines.len().saturating_sub(height);
        self.page = height.saturating_sub(1).max(1);
        if self.pinned_top.is_some_and(|top| top >= self.last_top) {
            self.follow();
        }
        let top = self.top();
        let below = lines.len().saturating_sub(top + height);
        let visible = lines.into_iter().skip(top).take(height).collect();
        (visible, below)
    }
}

impl Scrolled {
    pub fn new(lines: Vec<Line<'static>>, theme: Theme) -> Self {
        Self { lines, theme }
    }
}

impl StatefulWidget for Scrolled {
    type State = Scroll;

    fn render(self, area: Rect, buf: &mut Buffer, scroll: &mut Scroll) {
        let mut block = self.theme.bordered(Borders::TOP);
        let height = usize::from(block.inner(area).height);
        let (visible, below) = scroll.show(self.lines, height);
        if below > 0 {
            let more = format!(" ↓ {below} more · End ");
            block = block.title_top(Line::styled(more, self.theme.muted()).right_aligned());
        }
        Paragraph::new(visible).block(block).render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::test_support::{buffer_text, rendered_with};

    const HEIGHT: usize = 4;

    fn numbers(count: usize) -> Vec<usize> {
        (1..=count).collect()
    }

    fn shown(scroll: &mut Scroll, count: usize) -> (Vec<usize>, usize) {
        scroll.show(numbers(count), HEIGHT)
    }

    fn numbered_lines(count: usize) -> Vec<Line<'static>> {
        (1..=count)
            .map(|number| Line::raw(format!("line {number}")))
            .collect()
    }

    #[test]
    fn new_scroll_follows_the_end() {
        let mut scroll = Scroll::default();
        assert_eq!(shown(&mut scroll, 10), (vec![7, 8, 9, 10], 0));
        assert!(scroll.is_following());
    }

    #[test]
    fn short_content_is_shown_whole_and_cannot_be_scrolled() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 2);
        scroll.up(5);
        assert_eq!(shown(&mut scroll, 2), (vec![1, 2], 0));
        assert!(scroll.is_following());
    }

    #[test]
    fn up_and_down_move_by_rows_and_count_what_is_below() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 10);
        scroll.up(2);
        assert_eq!(shown(&mut scroll, 10), (vec![5, 6, 7, 8], 2));
        scroll.down(1);
        assert_eq!(shown(&mut scroll, 10), (vec![6, 7, 8, 9], 1));
    }

    #[test]
    fn reaching_the_end_follows_it_again() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 10);
        scroll.up(2);
        shown(&mut scroll, 10);
        scroll.down(5);
        assert!(scroll.is_following());
        assert_eq!(shown(&mut scroll, 12), (vec![9, 10, 11, 12], 0));
    }

    #[test]
    fn scrolled_view_stays_put_while_new_lines_arrive() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 10);
        scroll.up(3);
        assert_eq!(shown(&mut scroll, 10).0, [4, 5, 6, 7]);
        assert_eq!(shown(&mut scroll, 20), (vec![4, 5, 6, 7], 13));
    }

    #[test]
    fn page_moves_by_a_screen_keeping_one_row_of_context() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 20);
        scroll.page_up();
        assert_eq!(shown(&mut scroll, 20).0, [14, 15, 16, 17]);
        scroll.page_up();
        assert_eq!(shown(&mut scroll, 20).0, [11, 12, 13, 14]);
        scroll.page_down();
        assert_eq!(shown(&mut scroll, 20).0, [14, 15, 16, 17]);
    }

    #[test]
    fn start_and_follow_jump_to_the_ends() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 20);
        scroll.rewind();
        assert_eq!(shown(&mut scroll, 20), (vec![1, 2, 3, 4], 16));
        scroll.follow();
        assert_eq!(shown(&mut scroll, 20), (vec![17, 18, 19, 20], 0));
    }

    #[test]
    fn up_never_goes_above_the_first_line() {
        let mut scroll = Scroll::default();
        shown(&mut scroll, 10);
        scroll.up(100);
        assert_eq!(shown(&mut scroll, 10).0, [1, 2, 3, 4]);
    }

    #[test]
    fn widget_marks_how_much_is_below_only_when_scrolled_up() {
        let mut scroll = Scroll::default();
        let following = rendered_with(
            Scrolled::new(numbered_lines(10), Theme::default()),
            40,
            5,
            &mut scroll,
        );
        assert!(!buffer_text(&following).contains("more"));
        scroll.up(3);
        let scrolled = rendered_with(
            Scrolled::new(numbered_lines(10), Theme::default()),
            40,
            5,
            &mut scroll,
        );
        let text = buffer_text(&scrolled);
        assert!(text.contains("↓ 3 more · End"), "{text}");
        assert!(text.contains("line 4"), "{text}");
        assert!(!text.contains("line 10"), "{text}");
    }
}
