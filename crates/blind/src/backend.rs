use std::{
    io,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use tokio::runtime::Runtime;

use crate::conversation::{Conversation, Entry, ReplyEvent, Speaker};
use crate::providers::{Message, OpenAiCompatible};
use crate::runtime;

#[cfg(test)]
mod guarantees;

const ROOT_PROMPT: &str = "You are root, the conversational agent of the blind harness. \
You only talk with the user: you cannot read files, run commands or call other agents. \
The harness tells you below which agents this project has. That is all you know about the project: \
do not invent agents, contracts or file contents. \
When the user runs a chain of agents, its result appears in the conversation after the mark \
`[result of an agent chain the user ran]`. Answer in the user's language.";
const CHAIN_RESULT: &str = "[result of an agent chain the user ran]";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendEvent {
    Reply(ReplyEvent),
    Models(Result<Vec<String>, String>),
}

pub struct Backend {
    root_prompt: String,
    requests: Sender<Request>,
    events: Receiver<BackendEvent>,
}

enum Request {
    Ask(Vec<Message>),
    ListModels,
    UseModel(String),
}

struct Worker {
    runtime: Runtime,
    client: OpenAiCompatible,
    model: String,
    events: Sender<BackendEvent>,
}

impl Backend {
    pub fn start(client: OpenAiCompatible, model: String, agents: &str) -> io::Result<Self> {
        let (requests, pending) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let worker = Worker {
            runtime: runtime::single_threaded()?,
            client,
            model,
            events,
        };
        thread::spawn(move || worker.serve(&pending));
        Ok(Self {
            root_prompt: root_prompt(agents),
            requests,
            events: received,
        })
    }

    pub fn ask(&self, conversation: &Conversation) -> bool {
        self.send(Request::Ask(messages(&self.root_prompt, conversation)))
    }

    pub fn list_models(&self) -> bool {
        self.send(Request::ListModels)
    }

    pub fn use_model(&self, model: &str) -> bool {
        self.send(Request::UseModel(model.to_owned()))
    }

    pub fn poll(&self) -> Option<BackendEvent> {
        self.events.try_recv().ok()
    }

    fn send(&self, request: Request) -> bool {
        self.requests.send(request).is_ok()
    }
}

impl Worker {
    fn serve(mut self, pending: &Receiver<Request>) {
        for request in pending {
            match request {
                Request::Ask(messages) => self.answer(&messages),
                Request::ListModels => self.list_models(),
                Request::UseModel(model) => self.model = model,
            }
        }
    }

    fn answer(&self, messages: &[Message]) {
        let forward_text = |text: &str| self.reply(ReplyEvent::Text(text.to_owned()));
        let streaming = self
            .client
            .stream_answer(&self.model, messages, forward_text);
        match self.runtime.block_on(streaming) {
            Ok(usage) => self.reply(ReplyEvent::Done(usage)),
            Err(error) => self.reply(ReplyEvent::Failed(error.to_string())),
        }
    }

    fn list_models(&self) {
        let outcome = self.runtime.block_on(self.client.model_ids());
        self.emit(BackendEvent::Models(
            outcome.map_err(|error| error.to_string()),
        ));
    }

    fn reply(&self, event: ReplyEvent) {
        self.emit(BackendEvent::Reply(event));
    }

    fn emit(&self, event: BackendEvent) {
        self.events.send(event).ok();
    }
}

fn root_prompt(agents: &str) -> String {
    format!("{ROOT_PROMPT}\n\n{agents}")
}

fn messages(root_prompt: &str, conversation: &Conversation) -> Vec<Message> {
    let system = Message::system(root_prompt);
    let spoken = conversation.entries().iter().map(message);
    std::iter::once(system).chain(spoken).collect()
}

fn message(entry: &Entry) -> Message {
    match entry.speaker {
        Speaker::Human => Message::user(&entry.text),
        Speaker::Root => Message::assistant(&entry.text),
        Speaker::Chain => Message::user(&format!("{CHAIN_RESULT}\n{}", entry.text)),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Backend;
    use crate::providers::OpenAiCompatible;

    pub fn offline() -> Backend {
        let client = OpenAiCompatible::new("http://127.0.0.1:1", None);
        Backend::start(client, "test-model".to_owned(), "no agents").expect("backend starts")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use blind_core::Usage;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method, path},
    };

    use super::*;

    const PATIENCE: Duration = Duration::from_secs(5);
    const AGENTS: &str = "## explorer\nFinds the files a task is about.";

    fn next_event(backend: &Backend) -> BackendEvent {
        backend
            .events
            .recv_timeout(PATIENCE)
            .expect("an event in time")
    }

    fn asked(question: &str) -> Conversation {
        let mut conversation = Conversation::default();
        conversation.say(question.to_owned());
        conversation
    }

    fn backend_for(server: &MockServer) -> Backend {
        let client = OpenAiCompatible::new(&server.uri(), None);
        Backend::start(client, "glm-5.3".to_owned(), AGENTS).expect("backend starts")
    }

    fn one_word_answer() -> ResponseTemplate {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
            "data: [DONE]\n\n",
        );
        ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
    }

    #[tokio::test]
    async fn sends_the_root_prompt_with_the_history_and_forwards_the_answer() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"model": "glm-5.3", "messages": [
                {"role": "system", "content": root_prompt(AGENTS)},
                {"role": "user", "content": "hi"}
            ]})))
            .respond_with(one_word_answer())
            .mount(&server)
            .await;
        let backend = backend_for(&server);
        assert!(backend.ask(&asked("hi")));
        assert_eq!(
            next_event(&backend),
            BackendEvent::Reply(ReplyEvent::Text("Hello".to_owned()))
        );
        let usage = Usage {
            input_tokens: 7,
            output_tokens: 3,
        };
        assert_eq!(
            next_event(&backend),
            BackendEvent::Reply(ReplyEvent::Done(usage))
        );
    }

    #[tokio::test]
    async fn provider_failure_becomes_a_failed_reply() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;
        let backend = backend_for(&server);
        assert!(backend.ask(&asked("hi")));
        let failure = ReplyEvent::Failed("provider answered 401: unauthorized".to_owned());
        assert_eq!(next_event(&backend), BackendEvent::Reply(failure));
    }

    #[tokio::test]
    async fn later_questions_go_to_the_newly_chosen_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"model": "glm-5.3-flash"})))
            .respond_with(one_word_answer())
            .mount(&server)
            .await;
        let backend = backend_for(&server);
        assert!(backend.use_model("glm-5.3-flash"));
        assert!(backend.ask(&asked("hi")));
        assert_eq!(
            next_event(&backend),
            BackendEvent::Reply(ReplyEvent::Text("Hello".to_owned()))
        );
    }

    #[tokio::test]
    async fn lists_the_models_of_the_provider() {
        let server = MockServer::start().await;
        let models = json!({"data": [{"id": "glm-5.3"}, {"id": "beam"}]});
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(models))
            .mount(&server)
            .await;
        let backend = backend_for(&server);
        assert!(backend.list_models());
        let listed = vec!["beam".to_owned(), "glm-5.3".to_owned()];
        assert_eq!(next_event(&backend), BackendEvent::Models(Ok(listed)));
    }

    #[test]
    fn chain_result_reaches_root_as_a_marked_user_message() {
        let mut conversation = Conversation::default();
        conversation.delegate("check".to_owned());
        conversation.report("explorer\n{}".to_owned());
        assert_eq!(
            messages("prompt", &conversation),
            [
                Message::system("prompt"),
                Message::user("check"),
                Message::user("[result of an agent chain the user ran]\nexplorer\n{}"),
            ]
        );
    }

    #[test]
    fn history_is_sent_as_alternating_user_and_assistant_messages() {
        let mut conversation = asked("one");
        conversation.receive(ReplyEvent::Text("first".to_owned()));
        conversation.receive(ReplyEvent::Done(Usage::default()));
        conversation.say("two".to_owned());
        assert_eq!(
            messages(&root_prompt(AGENTS), &conversation),
            [
                Message::system(&root_prompt(AGENTS)),
                Message::user("one"),
                Message::assistant("first"),
                Message::user("two")
            ]
        );
    }
}
