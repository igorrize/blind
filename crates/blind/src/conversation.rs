use blind_core::Usage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    Human,
    Root,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub speaker: Speaker,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Waiting,
    Answering,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyEvent {
    Text(String),
    Done(Usage),
    Failed(String),
}

#[derive(Debug, Default)]
pub struct Conversation {
    entries: Vec<Entry>,
    status: Status,
    last_usage: Option<Usage>,
}

impl Conversation {
    pub fn say(&mut self, text: String) {
        self.entries.push(Entry {
            speaker: Speaker::Human,
            text,
        });
        self.status = Status::Waiting;
    }

    pub fn receive(&mut self, event: ReplyEvent) {
        match event {
            ReplyEvent::Text(text) => self.append_reply(&text),
            ReplyEvent::Done(usage) => {
                self.status = Status::Idle;
                self.last_usage = Some(usage);
            }
            ReplyEvent::Failed(reason) => self.status = Status::Failed(reason),
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.status, Status::Waiting | Status::Answering)
    }

    pub fn last_usage(&self) -> Option<Usage> {
        self.last_usage
    }

    fn append_reply(&mut self, text: &str) {
        if self.status == Status::Answering
            && let Some(reply) = self.entries.last_mut()
        {
            reply.text.push_str(text);
        } else {
            self.entries.push(Entry {
                speaker: Speaker::Root,
                text: text.to_owned(),
            });
        }
        self.status = Status::Answering;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(question: &str) -> Conversation {
        let mut conversation = Conversation::default();
        conversation.say(question.to_owned());
        conversation
    }

    fn text(part: &str) -> ReplyEvent {
        ReplyEvent::Text(part.to_owned())
    }

    fn texts(conversation: &Conversation) -> Vec<(Speaker, &str)> {
        conversation
            .entries()
            .iter()
            .map(|entry| (entry.speaker, entry.text.as_str()))
            .collect()
    }

    #[test]
    fn new_conversation_is_idle_and_empty() {
        let conversation = Conversation::default();
        assert!(conversation.entries().is_empty());
        assert!(!conversation.is_busy());
    }

    #[test]
    fn saying_something_waits_for_root() {
        let conversation = asked("hi");
        assert_eq!(texts(&conversation), [(Speaker::Human, "hi")]);
        assert_eq!(conversation.status(), &Status::Waiting);
        assert!(conversation.is_busy());
    }

    #[test]
    fn streamed_parts_grow_one_root_entry() {
        let mut conversation = asked("hi");
        conversation.receive(text("Hel"));
        conversation.receive(text("lo"));
        assert_eq!(
            texts(&conversation),
            [(Speaker::Human, "hi"), (Speaker::Root, "Hello")]
        );
        assert_eq!(conversation.status(), &Status::Answering);
    }

    #[test]
    fn done_frees_the_conversation_and_keeps_usage() {
        let usage = Usage {
            input_tokens: 7,
            output_tokens: 3,
        };
        let mut conversation = asked("hi");
        conversation.receive(text("Hello"));
        conversation.receive(ReplyEvent::Done(usage));
        assert!(!conversation.is_busy());
        assert_eq!(conversation.last_usage(), Some(usage));
    }

    #[test]
    fn second_answer_starts_a_new_root_entry() {
        let mut conversation = asked("one");
        conversation.receive(text("first"));
        conversation.receive(ReplyEvent::Done(Usage::default()));
        conversation.say("two".to_owned());
        conversation.receive(text("second"));
        assert_eq!(
            texts(&conversation),
            [
                (Speaker::Human, "one"),
                (Speaker::Root, "first"),
                (Speaker::Human, "two"),
                (Speaker::Root, "second")
            ]
        );
    }

    #[test]
    fn failure_is_kept_as_status_and_frees_the_conversation() {
        let mut conversation = asked("hi");
        conversation.receive(ReplyEvent::Failed("provider answered 401".to_owned()));
        assert_eq!(
            conversation.status(),
            &Status::Failed("provider answered 401".to_owned())
        );
        assert!(!conversation.is_busy());
    }
}
