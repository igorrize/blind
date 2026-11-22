use ratatui::crossterm::event::KeyEvent;

use super::draft::Draft;
use super::keys::{self, Action};
use super::scroll::Scroll;
use crate::backend::{Backend, BackendEvent};
use crate::chain_backend::ChainBackend;
use crate::chain_log::{ChainLog, RunEvent};
use crate::conversation::{Conversation, ReplyEvent};
use crate::crew::{Candidate, Crew};
use crate::model_catalog::Catalog;
use crate::root_model::RootModel;
use crate::ui_config::{Flavor, UiSettings};

const LINK_CLOSED: &str = "the connection to the model is closed, restart blind";
const RELOAD_KEY: char = 'r';
const SIDEBAR_KEY: char = 's';
const CHAIN_KEY: char = 'c';

#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Root,
    Settings,
    Chain,
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
    pub(super) chain: Vec<String>,
    pub(super) building: Option<Vec<String>>,
    pub(super) sending_to_chain: bool,
    pub(super) chain_log: ChainLog,
    pub(super) chat_scroll: Scroll,
    pub(super) chain_scroll: Scroll,
    crew: Crew,
    backend: Backend,
    chain_backend: ChainBackend,
}

pub struct Services {
    pub backend: Backend,
    pub chain_backend: ChainBackend,
    pub crew: Crew,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Root, Tab::Settings, Tab::Chain];
    pub const TITLES: [&'static str; 3] = ["root", "settings", "chain"];

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
    pub fn new(services: Services, root_model: RootModel, ui: UiSettings) -> Self {
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
            chain: Vec::new(),
            building: None,
            sending_to_chain: false,
            chain_log: ChainLog::default(),
            chat_scroll: Scroll::default(),
            chain_scroll: Scroll::default(),
            crew: services.crew,
            backend: services.backend,
            chain_backend: services.chain_backend,
        }
    }

    pub fn candidates(&self) -> Vec<Candidate> {
        let chain = self.building.as_deref().unwrap_or_default();
        self.crew.candidates(chain)
    }

    pub fn receive_events(&mut self) {
        while let Some(event) = self.backend.poll() {
            match event {
                BackendEvent::Reply(reply) => self.conversation.receive(reply),
                BackendEvent::Models(outcome) => self.models_arrived(outcome),
            }
        }
        while let Some(event) = self.chain_backend.poll() {
            self.chain_moved(event);
        }
    }

    fn chain_moved(&mut self, event: RunEvent) {
        if let Some(verdict) = event.verdict() {
            let report = format!("{}\n{verdict}", self.chain_log.title());
            self.conversation.report(report);
        }
        self.chain_log.receive(event);
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
        } else if self.building.is_some() {
            self.act_in_builder(action);
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
        } else if key == CHAIN_KEY {
            self.building = Some(self.chain.clone());
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

    fn act_in_builder(&mut self, action: Action) {
        match action {
            Action::Insert(key) => self.add_step(key),
            Action::DeleteBack => self.remove_last_step(),
            Action::Submit => self.use_built_chain(),
            Action::Back => self.building = None,
            Action::Up
            | Action::Down
            | Action::PageUp
            | Action::PageDown
            | Action::Home
            | Action::End
            | Action::SwitchTarget
            | Action::Quit
            | Action::Leader => {}
        }
    }

    fn add_step(&mut self, key: char) {
        let Some(chain) = self.building.as_mut() else {
            return;
        };
        let number = key
            .to_digit(10)
            .and_then(|digit| usize::try_from(digit).ok());
        if let Some(agent) = number.and_then(|number| self.crew.pick(chain, number)) {
            chain.push(agent);
        }
    }

    fn remove_last_step(&mut self) {
        if let Some(chain) = self.building.as_mut() {
            chain.pop();
        }
    }

    fn use_built_chain(&mut self) {
        self.chain = self.building.take().unwrap_or_default();
        self.sending_to_chain = !self.chain.is_empty();
    }

    fn act_in_sidebar(&mut self, cursor: Tab, action: Action) {
        match action {
            Action::Up => self.sidebar_cursor = Some(cursor.previous()),
            Action::Down => self.sidebar_cursor = Some(cursor.next()),
            Action::Submit => self.open(cursor),
            Action::Back => self.sidebar_cursor = None,
            Action::Insert(_)
            | Action::DeleteBack
            | Action::SwitchTarget
            | Action::PageUp
            | Action::PageDown
            | Action::Home
            | Action::End
            | Action::Quit
            | Action::Leader => {}
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
            Tab::Chain => {
                moved(&mut self.chain_scroll, action);
            }
        }
    }

    fn act_in_chat(&mut self, action: Action) {
        match action {
            Action::Insert(character) => self.draft.insert(character),
            Action::DeleteBack => self.draft.delete_back(),
            Action::Submit => self.send_draft(),
            Action::SwitchTarget => self.switch_target(),
            Action::Up
            | Action::Down
            | Action::PageUp
            | Action::PageDown
            | Action::Home
            | Action::End => {
                moved(&mut self.chat_scroll, action);
            }
            Action::Back | Action::Quit | Action::Leader => {}
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
            | Action::SwitchTarget
            | Action::PageUp
            | Action::PageDown
            | Action::Home
            | Action::End
            | Action::Quit
            | Action::Leader => {}
        }
    }

    fn switch_target(&mut self) {
        self.sending_to_chain = !self.sending_to_chain && !self.chain.is_empty();
    }

    fn send_draft(&mut self) {
        if self.sending_to_chain {
            self.send_to_chain();
        } else {
            self.send_to_root();
        }
    }

    fn send_to_chain(&mut self) {
        if self.chain_log.is_running() {
            return;
        }
        let Some(task) = self.draft.take() else {
            return;
        };
        self.conversation.delegate(task.clone());
        self.chain_log.start(&self.chain, &task);
        self.chain_scroll.follow();
        self.chat_scroll.follow();
        self.active = Tab::Chain;
        if !self.chain_backend.run(&self.chain, &task) {
            self.chain_moved(RunEvent::Failed(LINK_CLOSED.to_owned()));
        }
    }

    fn send_to_root(&mut self) {
        if self.conversation.is_busy() {
            return;
        }
        let Some(text) = self.draft.take() else {
            return;
        };
        self.conversation.say(text);
        self.chat_scroll.follow();
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

fn moved(scroll: &mut Scroll, action: Action) -> bool {
    match action {
        Action::Up => scroll.up(1),
        Action::Down => scroll.down(1),
        Action::PageUp => scroll.page_up(),
        Action::PageDown => scroll.page_down(),
        Action::Home => scroll.rewind(),
        Action::End => scroll.follow(),
        Action::Insert(_)
        | Action::DeleteBack
        | Action::Submit
        | Action::Back
        | Action::SwitchTarget
        | Action::Quit
        | Action::Leader => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyCode;
    use tempfile::TempDir;

    use super::*;
    use crate::backend::test_support::offline;
    use crate::chain_backend::test_support::unable_to_run;
    use crate::chain_log::Kind;
    use crate::conversation::Speaker;
    use crate::crew::test_support::explorer_planner_reviewer;
    use crate::root_model::test_support::saved_in;
    use crate::tui::keys::test_support::{ctrl, letter, plain};

    fn app(dir: &TempDir) -> App {
        app_with(dir, Crew::default())
    }

    fn app_with(dir: &TempDir, crew: Crew) -> App {
        let ui = UiSettings::load(dir.path().join("ui.toml")).expect("ui settings");
        let services = Services {
            backend: offline(),
            chain_backend: unable_to_run(),
            crew,
        };
        App::new(services, saved_in(dir), ui)
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
        press_all(&mut app, &[ctrl('x'), letter('s'), plain(KeyCode::Down)]);
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

    fn built(dir: &TempDir, keys: &[KeyEvent]) -> App {
        let mut app = app_with(dir, explorer_planner_reviewer());
        press_all(&mut app, &[ctrl('x'), letter('c')]);
        press_all(&mut app, keys);
        app
    }

    fn fitting(app: &App) -> Vec<String> {
        let candidates = app.candidates();
        let fitting = candidates
            .iter()
            .filter(|candidate| candidate.obstacle.is_none());
        fitting.map(|candidate| candidate.name.clone()).collect()
    }

    #[test]
    fn ctrl_x_c_opens_the_chain_builder_with_the_agents_that_can_go_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = built(&dir, &[]);
        assert_eq!(app.building.as_deref(), Some(&[][..]));
        assert_eq!(fitting(&app), ["explorer", "planner"]);
    }

    #[test]
    fn number_adds_a_step_and_the_candidates_follow_the_contracts() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = built(&dir, &[letter('1')]);
        assert_eq!(app.building.as_deref(), Some(&["explorer".to_owned()][..]));
        assert_eq!(fitting(&app), ["explorer", "planner", "reviewer"]);
    }

    #[test]
    fn agent_that_does_not_fit_cannot_be_added() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = built(&dir, &[letter('3'), letter('9'), letter('x')]);
        assert_eq!(app.building.as_deref(), Some(&[][..]));
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn backspace_removes_the_last_step() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = built(&dir, &[letter('1'), letter('3'), plain(KeyCode::Backspace)]);
        assert_eq!(app.building.as_deref(), Some(&["explorer".to_owned()][..]));
    }

    #[test]
    fn enter_keeps_the_chain_and_aims_the_next_message_at_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let app = built(&dir, &[letter('1'), letter('3'), plain(KeyCode::Enter)]);
        assert_eq!(app.building, None);
        assert_eq!(app.chain, ["explorer", "reviewer"]);
        assert!(app.sending_to_chain);
    }

    #[test]
    fn escape_closes_the_builder_and_keeps_the_previous_chain() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(
            &mut app,
            &[ctrl('x'), letter('c'), letter('3'), plain(KeyCode::Esc)],
        );
        assert_eq!(app.building, None);
        assert_eq!(app.chain, ["explorer"]);
    }

    #[test]
    fn builder_reopens_with_the_current_chain() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut app, &[ctrl('x'), letter('c')]);
        assert_eq!(app.building.as_deref(), Some(&["explorer".to_owned()][..]));
    }

    #[test]
    fn emptied_chain_sends_messages_to_root_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        let emptying = [
            ctrl('x'),
            letter('c'),
            plain(KeyCode::Backspace),
            plain(KeyCode::Enter),
        ];
        press_all(&mut app, &emptying);
        assert!(app.chain.is_empty());
        assert!(!app.sending_to_chain);
    }

    #[test]
    fn tab_switches_between_root_and_the_chain_only_when_a_chain_exists() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut without_chain = app(&dir);
        press_all(&mut without_chain, &[plain(KeyCode::Tab)]);
        assert!(!without_chain.sending_to_chain);
        let mut with_chain = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut with_chain, &[plain(KeyCode::Tab)]);
        assert!(!with_chain.sending_to_chain);
        press_all(&mut with_chain, &[plain(KeyCode::Tab)]);
        assert!(with_chain.sending_to_chain);
    }

    #[test]
    fn message_for_the_chain_starts_a_run_and_does_not_wake_root() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut app, &[letter('g'), letter('o'), plain(KeyCode::Enter)]);
        assert_eq!(app.active, Tab::Chain);
        assert!(app.chain_log.is_running());
        assert_eq!(app.chain_log.lines()[0].text, "explorer · go");
        assert_eq!(app.conversation.entries().len(), 1);
        assert_eq!(app.conversation.entries()[0].speaker, Speaker::Human);
        assert!(!app.conversation.is_busy());
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn end_of_a_run_is_reported_in_the_root_chat() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut app, &[letter('g'), plain(KeyCode::Enter)]);
        app.chain_moved(RunEvent::Completed {
            output: "{}".to_owned(),
            total: "chain · steps: 1".to_owned(),
        });
        assert!(!app.chain_log.is_running());
        let report = app.conversation.entries().last().expect("a report");
        assert_eq!(report.speaker, Speaker::Chain);
        assert_eq!(report.text, "explorer\n{}");
    }

    #[test]
    fn failure_to_prepare_the_chain_is_shown_in_the_log_and_the_chat() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut app, &[letter('g'), plain(KeyCode::Enter)]);
        while app.chain_log.is_running() {
            app.receive_events();
            std::thread::yield_now();
        }
        let last = app.chain_log.lines().last().expect("a line");
        assert_eq!(last.kind, Kind::Problem);
        assert_eq!(last.text, "failed: no agents here");
        let report = app.conversation.entries().last().expect("a report");
        assert_eq!(report.text, "explorer\nfailed: no agents here");
    }

    #[test]
    fn second_message_waits_until_the_running_chain_ends() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(&mut app, &[letter('a'), plain(KeyCode::Enter)]);
        press_all(
            &mut app,
            &[ctrl('x'), letter('1'), letter('b'), plain(KeyCode::Enter)],
        );
        assert_eq!(app.draft.text(), "b");
        assert_eq!(app.conversation.entries().len(), 1);
    }

    #[test]
    fn typing_in_the_chain_tab_does_not_reach_the_draft() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('3'), letter('h')]);
        assert_eq!(app.active, Tab::Chain);
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn arrows_and_page_keys_scroll_the_chat_and_end_follows_it_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[plain(KeyCode::Up)]);
        assert!(!app.chat_scroll.is_following());
        press_all(&mut app, &[plain(KeyCode::End)]);
        assert!(app.chat_scroll.is_following());
        press_all(&mut app, &[plain(KeyCode::PageUp), plain(KeyCode::Home)]);
        assert!(!app.chat_scroll.is_following());
        assert_eq!(app.draft.text(), "");
    }

    #[test]
    fn keys_in_the_chain_tab_scroll_the_chain_log_not_the_chat() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('3'), plain(KeyCode::PageUp)]);
        assert!(!app.chain_scroll.is_following());
        assert!(app.chat_scroll.is_following());
    }

    #[test]
    fn sending_a_message_returns_the_chat_to_its_end() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(
            &mut app,
            &[plain(KeyCode::Up), letter('h'), plain(KeyCode::Enter)],
        );
        assert!(app.chat_scroll.is_following());
    }

    #[test]
    fn new_chain_run_shows_its_log_from_the_end() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = built(&dir, &[letter('1'), plain(KeyCode::Enter)]);
        press_all(
            &mut app,
            &[
                ctrl('x'),
                letter('3'),
                plain(KeyCode::Up),
                ctrl('x'),
                letter('1'),
            ],
        );
        press_all(&mut app, &[letter('g'), plain(KeyCode::Enter)]);
        assert!(app.chain_scroll.is_following());
    }

    #[test]
    fn up_and_down_in_settings_still_move_the_cursor() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        let row = app.settings_row;
        press_all(&mut app, &[ctrl('x'), letter('2'), plain(KeyCode::Up)]);
        assert_eq!(app.settings_row, row - 1);
        assert!(app.chat_scroll.is_following());
    }

    #[test]
    fn ctrl_c_quits_from_any_tab() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut app = app(&dir);
        press_all(&mut app, &[ctrl('x'), letter('2')]);
        assert_eq!(app.handle(ctrl('c')), Flow::Quit);
    }
}
