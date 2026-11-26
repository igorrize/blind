use blind_core::Usage;

use super::{Conversation, ReplyEvent};

const TABLE: &str = "| file | lines |\n|------|-------|\n| a.rs | 12    |\n| b.rs | 7     |";

fn asked(conversation: &mut Conversation, prompt: &str) {
    conversation.say(prompt.to_owned());
}

fn answered(conversation: &mut Conversation, parts: &[&str]) {
    for part in parts {
        conversation.receive(ReplyEvent::Text((*part).to_owned()));
    }
    conversation.receive(ReplyEvent::Done(Usage::default()));
}

fn one_block(prompt: &str, answer: &str) -> Conversation {
    let mut conversation = Conversation::default();
    asked(&mut conversation, prompt);
    answered(&mut conversation, &[answer]);
    conversation
}

#[test]
fn empty_conversation_has_nothing_to_copy() {
    let conversation = Conversation::default();
    assert_eq!(conversation.last_answer(), None);
    assert_eq!(conversation.last_exchange(), None);
}

#[test]
fn finished_answer_is_copied_exactly_as_the_model_wrote_it() {
    let conversation = one_block("show the files as a table", TABLE);
    assert_eq!(conversation.last_answer(), Some(TABLE));
}

#[test]
fn exchange_is_the_prompt_an_empty_line_and_the_answer() {
    let conversation = one_block("show the files as a table", TABLE);
    let expected = format!("show the files as a table\n\n{TABLE}");
    assert_eq!(conversation.last_exchange(), Some(expected));
}

#[test]
fn answer_that_arrived_in_parts_is_copied_whole() {
    let mut conversation = Conversation::default();
    asked(&mut conversation, "greet me");
    answered(&mut conversation, &["Hel", "lo, ", "world"]);
    assert_eq!(conversation.last_answer(), Some("Hello, world"));
}

#[test]
fn latest_finished_block_is_the_one_copied() {
    let mut conversation = one_block("first", "one");
    asked(&mut conversation, "second");
    answered(&mut conversation, &["two"]);
    assert_eq!(conversation.last_answer(), Some("two"));
    assert_eq!(
        conversation.last_exchange(),
        Some("second\n\ntwo".to_owned())
    );
}

#[test]
fn answer_still_being_printed_is_not_finished() {
    let mut conversation = one_block("first", "one");
    asked(&mut conversation, "second");
    conversation.receive(ReplyEvent::Text("tw".to_owned()));
    assert_eq!(conversation.last_answer(), Some("one"));
    assert_eq!(
        conversation.last_exchange(),
        Some("first\n\none".to_owned())
    );
}

#[test]
fn prompt_still_waiting_for_its_answer_leaves_the_previous_block() {
    let mut conversation = one_block("first", "one");
    asked(&mut conversation, "second");
    assert_eq!(conversation.last_answer(), Some("one"));
    assert_eq!(
        conversation.last_exchange(),
        Some("first\n\none".to_owned())
    );
}

#[test]
fn first_answer_still_being_printed_leaves_nothing_to_copy() {
    let mut conversation = Conversation::default();
    asked(&mut conversation, "first");
    conversation.receive(ReplyEvent::Text("on".to_owned()));
    assert_eq!(conversation.last_answer(), None);
    assert_eq!(conversation.last_exchange(), None);
}
