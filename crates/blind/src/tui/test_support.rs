use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};

pub fn rendered(widget: impl Widget, width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    widget.render(area, &mut buffer);
    buffer
}

pub fn rendered_text(widget: impl Widget, width: u16, height: u16) -> String {
    buffer_text(&rendered(widget, width, height))
}

pub fn buffer_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

pub fn style_of(buffer: &Buffer, text: &str) -> Style {
    let symbols: Vec<&str> = buffer.content().iter().map(|cell| cell.symbol()).collect();
    let wanted: Vec<String> = text.chars().map(String::from).collect();
    let start = symbols
        .windows(wanted.len())
        .position(|window| window == wanted)
        .unwrap_or_else(|| panic!("{text:?} is not on the screen"));
    buffer.content()[start].style()
}
