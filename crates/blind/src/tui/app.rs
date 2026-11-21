use ratatui::crossterm::event::KeyEvent;

use super::draft::Draft;
use super::keys::{self, Action};
use crate::backend::{Backend, BackendEvent};
use crate::conversation::{Conversation, ReplyEvent};
use crate::model_catalog::Catalog;
use crate::root_model::RootModel;
use crate::ui_config::{Flavor, UiSettings};

const LINK_CLOSED: &str = "the connection to the model is closed, restart blind";
const RELOAD_KEY: char = 'r';
const SIDEBAR_KEY: char = 's';

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Root,
    Settings,
}

pub struct App {
    pub(super) conversation: Conversation,
    pub(super) draft: Draft,
    pub(super) catalog: Catalog,
    pub(super) root_model: RootModel,
    pub(super) ui: UiSettings,
    pub(super) settings_row: usize,
    pub(super) active: Tab,
    pub(super) awaiting_command: bool,
    pub(super) sidebar_cursor: Option<Tab>,
    pub(super) notice: Option<String>,
    backend: Backend,
}

impl Tab {
    pub const ALL: [Tab; 2] = [Tab::Root, Tab::Settings];
    pub const TITLES: [&'static str; 2] = ["root", "settings"];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn title(self) -> &'static str {
        Self::TITLES[self.index()]
    }

    fn previous(self) -> Tab {
        Self::ALL[self.index().saturating_sub(1)]
    }

    fn next(self) -> Tab {
        Self::ALL.get(self.index() + 1).copied().unwrap_or(self)
    }

    fn numbered(digit: char) -> Option<Tab> {
        let number = usize::try_from(digit.to_digit(10)?).ok()?;
        Self::ALL.get(number.checked_sub(1)?).copied()
    }
}

impl App {
    pub fn new(backend: Backend, root_model: RootModel, ui: UiSettings) -> Self {
        Self {
            conversation: Conversation::default(),
            draft: Draft::default(),
            catalog: Catalog::default(),
            root_model,
            settings_row: ui.flavor().index(),
            ui,
            active: Tab::Root,
            awaiting_command: false,
            sidebar_cursor: None,
            notice: None,
            backend,
        }
    }

    pub fn receive_events(&mut self) {
        while let Some(event) = self.backend.poll() {
            match event {
                BackendEvent::Reply(reply) => self.conversation.receive(reply),
                BackendEvent::Models(outcome) => self.models_arrived(outcome),
            }
        }
    }

    fn models_arrived(&mut self, outcome: Result<Vec<String>, String>) {
        self.catalog = outcome.into();
        self.settings_row = self.settings_row.min(self.last_settings_row());
    }

    fn last_settings_row(&self) -> usize {
        Flavor::ALL.len() + self.catalog.models().len() - 1
    }

    pub fn handle(&mut self, key: KeyEvent) -> Flow {
        let Some(action) = keys::action_for(key) else {
            return Flow::Continue;
        };
        if action == Action::Quit {
            return Flow::Quit;
        }
        if self.awaiting_command {
            self.run_command(action);
        } else if action == Action::Leader {
            self.awaiting_command = true;
        } else if let Some(cursor) = self.sidebar_cursor {
            self.act_in_sidebar(cursor, action);
        } else {
            self.act_in_tab(action);
        }
        Flow::Continue
    }

    fn run_command(&mut self, action: Action) {
        self.awaiting_command = false;
        let Action::Insert(key) = action else {
            return;
        };
        if key == SIDEBAR_KEY {
            self.toggle_sidebar();
        } else if let Some(tab) = Tab::numbered(key) {
            self.open(tab);
        }
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_cursor = match self.sidebar_cursor {
            Some(_) => None,
            None => Some(self.active),
        };
    }

    fn act_in_sidebar(&mut self, cursor: Tab, action: Action) {
        match action {
            Action::Up => self.sidebar_cursor = Some(cursor.previous()),
            Action::Down => self.sidebar_cursor = Some(cursor.next()),
            Action::Submit => self.open(cursor),
            Action::Back => self.sidebar_cursor = None,
            Action::Insert(_) | Action::DeleteBack | Action::Quit | Action::Leader => {}
        }
    }

    fn open(&mut self, tab: Tab) {
        self.active = tab;
        self.sidebar_cursor = None;
        if tab == Tab::Settings && self.catalog.needs_loading() {
            self.load_models();
        }
    }

    fn act_in_tab(&mut self, action: Action) {
        match self.active {
            Tab::Root => self.act_in_chat(action),
            Tab::Settings => self.act_in_settings(action),
        }
    }

    fn act_in_chat(&mut self, action: Action) {
        match action {
            Action::Insert(character) => self.draft.insert(character),
            Action::DeleteBack => self.draft.delete_back(),
            Action::Submit => self.send_draft(),
            Action::Up | Action::Down | Action::Back | Action::Quit | Action::Leader => {}
        }
    }

    fn act_in_settings(&mut self, action: Action) {
        match action {
            Action::Up => self.settings_row = self.settings_row.saturating_sub(1),
            Action::Down => {
                self.settings_row = (self.settings_row + 1).min(self.last_settings_row())
            }
            Action::Submit => self.apply_setting(),
            Action::Insert(RELOAD_KEY) => self.load_models(),
            Action::Insert(_)
            | Action::DeleteBack
            | Action::Back
            | Action::Quit
            | Action::Leader => {}
        }
    }

    fn send_draft(&mut self) {
        if self.conversation.is_busy() {
            return;
        }
        let Some(text) = self.draft.take() else {
            return;
        };
        self.conversation.say(text);
        if !self.backend.ask(&self.conversation) {
            self.conversation
                .receive(ReplyEvent::Failed(LINK_CLOSED.to_owned()));
        }
    }

    fn load_models(&mut self) {
        self.catalog = Catalog::Loading;
        if !self.backend.list_models() {
            self.models_arrived(Err(LINK_CLOSED.to_owned()));
        }
    }

    fn apply_setting(&mut self) {
        match Flavor::ALL.get(self.settings_row) {
            Some(flavor) => self.choose_flavor(*flavor),
            None => self.choose_model(self.settings_row - Flavor::ALL.len()),
        }
    }

    fn choose_flavor(&mut self, flavor: Flavor) {
        let notice = match self.ui.choose(flavor) {
            Ok(()) => format!("theme is now {}", flavor.name()),
            Err(error) => format!("not saved: {error:#}"),
        };
        self.notice = Some(notice);
    }

    fn choose_model(&mut self, index: usize) {
        let Some(model) = self.catalog.models().get(index).cloned() else {
            return;
        };
        let notice = match self.root_model.choose(&model) {
            Ok(()) if self.backend.use_model(&model) => {
                format!("root now uses {}", self.root_model.current())
            }
            Ok(()) => LINK_CLOSED.to_owned(),
            Err(error) => format!("not saved: {error:#}"),
        };
        self.notice = Some(notice);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;
    use tempfile::TempDir;

    use super::*;
    use crate::backend::test_support::offline;
    use crate::root_model::test_support::saved_in;
    use crate::tui::keys::test_support::{ctrl, letter, plain};

    fn app(dir: &TempDir) -> App {
        let ui = UiSettings::load(dir.path().join("ui.toml")).expect("ui settings");
        App::new(offline(), saved_in(dir), ui)
    }

    fn press_all(app: &mut App, keys: &[KeyEvent]) {
        for key in keys {
            assert_eq!(app.handle(*key), Flow::Continue);
        }
    }

    fn models_arrived(app: &mut App, models: &[&str]) {
        let models = models.iter().map(|model| (*model).to_owned()).collect();
        app.models_arrived(Ok(models));
    }

    #[test]
    fn tab_numbers_match_their_order() {
        for tab in Tab::ALL {
            assert_eq!(Tab::ALL[tab.index()], tab);
        }
        assert_eq!(Tab::numbered('2'), Some(Tab::Settings));
        assert_eq!(Tab::numbered('0'), None);
        assert_eq!(Tab::numbered('9'), None);
    }

    #[test]
    fn typing_in_the_root_tab_fills_the_draft() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(
            &mut app,
            &[letter('h'), letter('i'), plain(KeyCode::Backspace)],
        );
        assert_eq!(app.draft.text(), "h");
    }

    #[test]
    fn enter_moves_the_draft_into_the_conversation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[letter('h'), letter('i'), plain(KeyCode::Enter)]);
        assert_eq!(app.draft.text(), "");
        assert_eq!(app.conversation.entries().len(), 1);
        assert!(app.conversation.is_busy());
    }

    #[test]
    fn ctrl_x_then_a_number_switches_the_tab_and_loads_models() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x')]);
        assert!(app.awaiting_command);
        press_all(&mut app, &[letter('2')]);
        assert_eq!(app.active, Tab::Settings);
        assert!(!app.awaiting_command);
        assert!(!app.catalog.needs_loading());
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn ctrl_x_then_another_key_cancels_without_typing() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('x')]);
        assert_eq!(app.active, Tab::Root);
        assert!(!app.awaiting_command);
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn letters_in_settings_do_not_reach_the_draft() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('2'), letter('h')]);
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn choosing_a_model_in_settings_switches_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('2')]);
        models_arrived(&mut app, &["glm-5.3", "glm-5.3-flash"]);
        let down = plain(KeyCode::Down);
        press_all(&mut app, &[down, down, plain(KeyCode::Enter)]);
        assert_eq!(
            app.root_model.current().to_string(),
            "ollama-cloud/glm-5.3-flash"
        );
        assert_eq!(
            app.notice.as_deref(),
            Some("root now uses ollama-cloud/glm-5.3-flash")
        );
    }

    #[test]
    fn settings_cursor_starts_on_the_current_theme() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = app(&dir);
        assert_eq!(Flavor::ALL[app.settings_row], app.ui.flavor());
    }

    #[test]
    fn choosing_a_theme_in_settings_switches_and_saves_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        let up = plain(KeyCode::Up);
        press_all(
            &mut app,
            &[
                ctrl('x'),
                letter('2'),
                up,
                up,
                up,
                up,
                plain(KeyCode::Enter),
            ],
        );
        assert_eq!(app.ui.flavor(), Flavor::Latte);
        assert_eq!(app.notice.as_deref(), Some("theme is now latte"));
        let saved = std::fs::read_to_string(dir.path().join("ui.toml")).expect("ui.toml");
        assert!(saved.contains(r#"flavor = "latte""#), "{saved}");
    }

    #[test]
    fn settings_cursor_stops_at_the_last_row() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('2')]);
        models_arrived(&mut app, &["glm-5.3"]);
        let down = plain(KeyCode::Down);
        press_all(&mut app, &[down, down, down]);
        assert_eq!(app.settings_row, Flavor::ALL.len());
    }

    #[test]
    fn ctrl_x_s_puts_the_cursor_on_the_open_tab_in_the_sidebar() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('s')]);
        assert_eq!(app.sidebar_cursor, Some(Tab::Root));
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn arrows_and_enter_in_the_sidebar_open_a_tab_and_return_the_focus() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(
            &mut app,
            &[
                ctrl('x'),
                letter('s'),
                plain(KeyCode::Down),
                plain(KeyCode::Down),
            ],
        );
        assert_eq!(app.sidebar_cursor, Some(Tab::Settings));
        press_all(&mut app, &[plain(KeyCode::Enter)]);
        assert_eq!(app.active, Tab::Settings);
        assert_eq!(app.sidebar_cursor, None);
    }

    #[test]
    fn escape_leaves_the_sidebar_without_switching_the_tab() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(
            &mut app,
            &[
                ctrl('x'),
                letter('s'),
                plain(KeyCode::Down),
                plain(KeyCode::Esc),
            ],
        );
        assert_eq!(app.active, Tab::Root);
        assert_eq!(app.sidebar_cursor, None);
    }

    #[test]
    fn typing_in_the_sidebar_does_not_reach_the_draft() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(
            &mut app,
            &[ctrl('x'), letter('s'), letter('h'), plain(KeyCode::Up)],
        );
        assert_eq!(app.draft.text(), "");
        assert_eq!(app.sidebar_cursor, Some(Tab::Root));
    }

    #[test]
    fn ctrl_c_quits_from_any_tab() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('2')]);
        assert_eq!(app.handle(ctrl('c')), Flow::Quit);
    }
}
