mod app;
mod chat_view;
mod draft;
mod keys;
mod layout;
mod settings_view;
mod sidebar;
mod status_line;
mod tab_bar;
mod theme;

#[cfg(test)]
mod test_support;

use std::time::Duration;

use app::{App, Flow, Tab};
use chat_view::ChatView;
use layout::ScreenLayout;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event},
    layout::Rect,
    widgets::Block,
};
use settings_view::SettingsView;
use sidebar::Sidebar;
use status_line::StatusLine;
use tab_bar::TabBar;
use theme::Theme;

use crate::backend::Backend;
use crate::root_model::RootModel;
use crate::ui_config::UiSettings;

const TICK: Duration = Duration::from_millis(50);
const KEYS_HINT: &str = "Ctrl-X commands · Ctrl-C quit";
const COMMAND_HINT: &str = "1 root · 2 settings · s sidebar";
const SIDEBAR_HINT: &str = "↑↓ move · Enter open · Esc back";

pub fn run(backend: Backend, root_model: RootModel, ui: UiSettings) -> anyhow::Result<()> {
    let mut app = App::new(backend, root_model, ui);
    let mut terminal = ratatui::init();
    let result = run_until_quit(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run_until_quit(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    loop {
        app.receive_events();
        terminal.draw(|frame| draw(frame, app))?;
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && app.handle(key) == Flow::Quit
        {
            return Ok(());
        }
    }
}

fn draw(frame: &mut Frame, app: &App) {
    let theme = Theme::of(app.ui.flavor());
    let layout = ScreenLayout::split(frame.area());
    let model = app.root_model.current().to_string();
    frame.render_widget(Block::default().style(theme.screen()), frame.area());
    frame.render_widget(
        Sidebar::new(app.active, app.sidebar_cursor, theme),
        layout.sidebar,
    );
    frame.render_widget(
        TabBar::new(&Tab::TITLES, app.active.index(), theme),
        layout.tab_bar,
    );
    draw_active_tab(frame, app, theme, layout.content);
    frame.render_widget(
        StatusLine::new(&model, app.conversation.last_usage(), hint(app), theme),
        layout.status_line,
    );
}

fn draw_active_tab(frame: &mut Frame, app: &App, theme: Theme, area: Rect) {
    match app.active {
        Tab::Root => {
            let chat = ChatView::new(&app.conversation, app.draft.text(), theme);
            frame.render_widget(chat, area);
        }
        Tab::Settings => {
            let settings = SettingsView {
                theme,
                flavor: app.ui.flavor(),
                catalog: &app.catalog,
                current_model: app.root_model.current().model(),
                row: app.settings_row,
                notice: app.notice.as_deref(),
            };
            frame.render_widget(settings, area);
        }
    }
}

fn hint(app: &App) -> &'static str {
    if app.awaiting_command {
        COMMAND_HINT
    } else if app.sidebar_cursor.is_some() {
        SIDEBAR_HINT
    } else {
        KEYS_HINT
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, crossterm::event::KeyCode};
    use tempfile::TempDir;

    use super::*;
    use crate::backend::test_support::offline;
    use crate::root_model::test_support::saved_in;
    use crate::tui::keys::test_support::{ctrl, letter, plain};
    use crate::ui_config::Flavor;
    use test_support::buffer_text;

    fn app(dir: &TempDir) -> App {
        let ui = UiSettings::load(dir.path().join("ui.toml")).expect("ui settings");
        App::new(offline(), saved_in(dir), ui)
    }

    fn drawn(app: &App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn screen(app: &App) -> String {
        buffer_text(&drawn(app))
    }

    fn assert_shows(screen: &str, expected: &[&str]) {
        for text in expected {
            assert!(screen.contains(text), "missing {text:?}");
        }
    }

    #[test]
    fn root_tab_shows_the_chat_and_every_region() {
        let dir = tempfile::tempdir().expect("temp dir");
        let screen = screen(&app(&dir));
        assert_shows(
            &screen,
            &[
                "AGENTS",
                "[ root ]",
                "settings",
                "Type a message",
                "ollama-cloud/glm-5.3",
                KEYS_HINT,
            ],
        );
    }

    #[test]
    fn typed_text_is_visible_in_the_input() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(letter('o'));
        app.handle(letter('k'));
        assert_shows(&screen(&app), &["› ok"]);
    }

    #[test]
    fn ctrl_x_shows_the_commands_and_a_number_opens_settings() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(ctrl('x'));
        assert_shows(&screen(&app), &[COMMAND_HINT]);
        app.handle(letter('2'));
        assert_shows(
            &screen(&app),
            &["[ settings ]", "Theme", "› mocha", "Root model", KEYS_HINT],
        );
    }

    #[test]
    fn ctrl_x_s_moves_the_focus_to_the_sidebar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(ctrl('x'));
        app.handle(letter('s'));
        assert_shows(&screen(&app), &["› root", SIDEBAR_HINT]);
    }

    #[test]
    fn whole_screen_is_painted_with_the_theme_background() {
        let dir = tempfile::tempdir().expect("temp dir");
        let buffer = drawn(&app(&dir));
        let theme = Theme::of(Flavor::Mocha);
        let backgrounds = [theme.screen().bg, theme.sidebar().bg, theme.active_tab().bg];
        let unpainted = buffer
            .content()
            .iter()
            .filter(|cell| !backgrounds.contains(&cell.style().bg))
            .count();
        assert_eq!(unpainted, 0);
    }

    #[test]
    fn choosing_another_flavor_repaints_the_screen() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        let up = plain(KeyCode::Up);
        for key in [ctrl('x'), letter('2'), up, up, up, plain(KeyCode::Enter)] {
            app.handle(key);
        }
        let content = drawn(&app);
        let latte = Theme::of(Flavor::Latte).screen().bg;
        let painted_latte = content
            .content()
            .iter()
            .filter(|cell| cell.style().bg == latte)
            .count();
        assert!(painted_latte > 0);
    }
}
