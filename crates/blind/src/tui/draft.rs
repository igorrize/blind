#[derive(Default)]
pub struct Draft {
    text: String,
}

impl Draft {
    pub fn insert(&mut self, character: char) {
        self.text.push(character);
    }

    pub fn delete_back(&mut self) {
        self.text.pop();
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn take(&mut self) -> Option<String> {
        if self.text.trim().is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Draft {
        let mut draft = Draft::default();
        text.chars().for_each(|character| draft.insert(character));
        draft
    }

    #[test]
    fn typing_and_deleting_edit_the_end_of_the_text() {
        let mut draft = typed("привет!");
        draft.delete_back();
        assert_eq!(draft.text(), "привет");
    }

    #[test]
    fn deleting_from_an_empty_draft_does_nothing() {
        let mut draft = Draft::default();
        draft.delete_back();
        assert_eq!(draft.text(), "");
    }

    #[test]
    fn taking_the_text_leaves_the_draft_empty() {
        let mut draft = typed("hi");
        assert_eq!(draft.take(), Some("hi".to_owned()));
        assert_eq!(draft.text(), "");
    }

    #[test]
    fn blank_draft_cannot_be_taken() {
        let mut draft = typed("   ");
        assert_eq!(draft.take(), None);
        assert_eq!(draft.text(), "   ");
    }
}
