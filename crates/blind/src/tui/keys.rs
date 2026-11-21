use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Leader,
    Submit,
    DeleteBack,
    Up,
    Down,
    Back,
    Insert(char),
}

pub fn action_for(key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if is_ctrl(key, 'c') {
        return Some(Action::Quit);
    }
    if is_ctrl(key, 'x') {
        return Some(Action::Leader);
    }
    named_key(key.code).or_else(|| typed_character(key).map(Action::Insert))
}

fn is_ctrl(key: KeyEvent, letter: char) -> bool {
    key.code == KeyCode::Char(letter) && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn named_key(code: KeyCode) -> Option<Action> {
    [
        (KeyCode::Enter, Action::Submit),
        (KeyCode::Backspace, Action::DeleteBack),
        (KeyCode::Up, Action::Up),
        (KeyCode::Down, Action::Down),
        (KeyCode::Esc, Action::Back),
    ]
    .into_iter()
    .find_map(|(named, action)| (named == code).then_some(action))
}

fn typed_character(key: KeyEvent) -> Option<char> {
    let KeyCode::Char(character) = key.code else {
        return None;
    };
    let shortcut = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
    (!shortcut).then_some(character)
}

#[cfg(test)]
pub(crate) mod test_support {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    pub fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    pub fn ctrl(letter: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(letter), KeyModifiers::CONTROL)
    }

    pub fn letter(character: char) -> KeyEvent {
        plain(KeyCode::Char(character))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{ctrl, letter, plain};
    use super::*;

    #[test]
    fn ctrl_c_quits_and_ctrl_x_starts_a_command() {
        assert_eq!(action_for(ctrl('c')), Some(Action::Quit));
        assert_eq!(action_for(ctrl('x')), Some(Action::Leader));
    }

    #[test]
    fn letters_are_typed_including_q_and_uppercase() {
        assert_eq!(action_for(letter('q')), Some(Action::Insert('q')));
        let shifted = KeyEvent::new(KeyCode::Char('Я'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shifted), Some(Action::Insert('Я')));
    }

    #[test]
    fn named_keys_have_their_actions() {
        assert_eq!(action_for(plain(KeyCode::Enter)), Some(Action::Submit));
        assert_eq!(
            action_for(plain(KeyCode::Backspace)),
            Some(Action::DeleteBack)
        );
        assert_eq!(action_for(plain(KeyCode::Up)), Some(Action::Up));
        assert_eq!(action_for(plain(KeyCode::Down)), Some(Action::Down));
        assert_eq!(action_for(plain(KeyCode::Esc)), Some(Action::Back));
    }

    #[test]
    fn other_shortcuts_and_keys_do_nothing() {
        assert_eq!(action_for(ctrl('y')), None);
        assert_eq!(action_for(plain(KeyCode::Tab)), None);
    }

    #[test]
    fn releasing_a_key_does_nothing() {
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(action_for(release), None);
    }
}
