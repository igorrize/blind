mod app;
mod chain_builder;
mod chain_view;
mod chat_view;
mod draft;
mod keys;
mod layout;
mod scroll;
mod settings_view;
mod sidebar;
mod status_line;
mod tab_bar;
mod theme;
mod wrap;

#[cfg(test)]
mod test_support;

use std::time::Duration;

pub use app::Services;
use app::{App, Flow, Tab};
use chain_builder::{ARROW, ChainBuilder};
use chain_view::ChainView;
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

use crate::root_model::RootModel;
use crate::ui_config::UiSettings;

const TICK: Duration = Duration::from_millis(50);
const KEYS_HINT: &str = "Ctrl-X commands · Ctrl-C quit";
const READING_HINT: &str = "↑↓ PgUp PgDn Home End scroll · Ctrl-X commands · Ctrl-C quit";
const COMMAND_HINT: &str = "1 root · 2 settings · 3 chain · c build chain · s sidebar";
const BUILDER_HINT: &str = "1-9 add step · Backspace remove · Enter use · Esc cancel";
const SIDEBAR_HINT: &str = "↑↓ move · Enter open · Esc back";

pub fn run(services: Services, root_model: RootModel, ui: UiSettings) -> anyhow::Result<()> {
    let mut app = App::new(services, root_model, ui);
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

fn draw(frame: &mut Frame, app: &mut App) {
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
    if let Some(chain) = &app.building {
        let candidates = app.candidates();
        frame.render_widget(ChainBuilder::new(chain, &candidates, theme), layout.content);
    }
}

fn target(app: &App) -> Option<String> {
    if app.chain.is_empty() {
        return None;
    }
    let chain = app.chain.join(ARROW);
    Some(if app.sending_to_chain {
        format!("to chain: {chain} · Tab: talk to root")
    } else {
        format!("to root · Tab: send to chain {chain}")
    })
}

fn draw_active_tab(frame: &mut Frame, app: &mut App, theme: Theme, area: Rect) {
    match app.active {
        Tab::Root => {
            let target = target(app);
            let chat = ChatView::new(&app.conversation, app.draft.text(), theme)
                .sending_to(target.as_deref());
            frame.render_stateful_widget(chat, area, &mut app.chat_scroll);
        }
        Tab::Chain => {
            let log = ChainView::new(&app.chain_log, theme);
            frame.render_stateful_widget(log, area, &mut app.chain_scroll);
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
    } else if app.building.is_some() {
        BUILDER_HINT
    } else if app.sidebar_cursor.is_some() {
        SIDEBAR_HINT
    } else if app.active == Tab::Settings {
        KEYS_HINT
    } else {
        READING_HINT
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, crossterm::event::KeyCode};
    use tempfile::TempDir;

    use super::*;
    use crate::backend::test_support::offline;
    use crate::chain_backend::test_support::unable_to_run;
    use crate::crew::test_support::explorer_planner_reviewer;
    use crate::root_model::test_support::saved_in;
    use crate::tui::keys::test_support::{ctrl, letter, plain};
    use crate::ui_config::Flavor;
    use test_support::buffer_text;

    fn app(dir: &TempDir) -> App {
        let ui = UiSettings::load(dir.path().join("ui.toml")).expect("ui settings");
        let services = Services {
            backend: offline(),
            chain_backend: unable_to_run(),
            crew: explorer_planner_reviewer(),
        };
        App::new(services, saved_in(dir), ui)
    }

    fn drawn(app: &mut App) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("test terminal");
        terminal.draw(|frame| draw(frame, app)).expect("draw");
        terminal.backend().buffer().clone()
    }

    fn screen(app: &mut App) -> String {
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
        let screen = screen(&mut app(&dir));
        assert_shows(
            &screen,
            &[
                "AGENTS",
                "[ root ]",
                "settings",
                "Type a message",
                "ollama-cloud/glm-5.3",
                READING_HINT,
            ],
        );
    }

    #[test]
    fn typed_text_is_visible_in_the_input() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(letter('o'));
        app.handle(letter('k'));
        assert_shows(&screen(&mut app), &["› ok"]);
    }

    #[test]
    fn ctrl_x_shows_the_commands_and_a_number_opens_settings() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(ctrl('x'));
        assert_shows(&screen(&mut app), &[COMMAND_HINT]);
        app.handle(letter('2'));
        assert_shows(
            &screen(&mut app),
            &["[ settings ]", "Theme", "› mocha", "Root model", KEYS_HINT],
        );
    }

    #[test]
    fn ctrl_x_s_moves_the_focus_to_the_sidebar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(ctrl('x'));
        app.handle(letter('s'));
        assert_shows(&screen(&mut app), &["› root", SIDEBAR_HINT]);
    }

    #[test]
    fn ctrl_x_c_shows_the_chain_builder_over_the_chat() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        app.handle(ctrl('x'));
        app.handle(letter('c'));
        assert_shows(
            &screen(&mut app),
            &[
                " chain ",
                " 1  explorer",
                " 2  planner",
                "cannot go first: needs files",
                BUILDER_HINT,
            ],
        );
    }

    #[test]
    fn built_chain_is_shown_above_the_input_with_the_way_back_to_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        let keys = [
            ctrl('x'),
            letter('c'),
            letter('1'),
            letter('3'),
            plain(KeyCode::Enter),
        ];
        for key in keys {
            app.handle(key);
        }
        assert_shows(
            &screen(&mut app),
            &["to chain: explorer → reviewer · Tab: talk to root"],
        );
        app.handle(plain(KeyCode::Tab));
        assert_shows(
            &screen(&mut app),
            &["to root · Tab: send to chain explorer → reviewer"],
        );
    }

    #[test]
    fn message_for_the_chain_opens_the_chain_tab_with_the_run() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        let keys = [
            ctrl('x'),
            letter('c'),
            letter('1'),
            plain(KeyCode::Enter),
            letter('g'),
            letter('o'),
            plain(KeyCode::Enter),
        ];
        for key in keys {
            app.handle(key);
        }
        assert_shows(&screen(&mut app), &["[ chain ]", "explorer · go"]);
    }

    #[test]
    fn whole_screen_is_painted_with_the_theme_background() {
        let dir = tempfile::tempdir().expect("temp dir");
        let buffer = drawn(&mut app(&dir));
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
        let content = drawn(&mut app);
        let latte = Theme::of(Flavor::Latte).screen().bg;
        let painted_latte = content
            .content()
            .iter()
            .filter(|cell| cell.style().bg == latte)
            .count();
        assert!(painted_latte > 0);
    }
}
