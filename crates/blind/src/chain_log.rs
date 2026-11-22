const ARROW: &str = " → ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    StepStarted(String),
    Text(String),
    ToolCalled {
        call: String,
        refusal: Option<String>,
    },
    Note(String),
    StepResult(String),
    StepFinished(String),
    Completed {
        output: String,
        total: String,
    },
    Stopped(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Run,
    Step,
    Said,
    Called,
    Refused,
    Note,
    Result,
    Summary,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub kind: Kind,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct ChainLog {
    lines: Vec<LogLine>,
    title: String,
    running: bool,
}

impl RunEvent {
    pub fn verdict(&self) -> Option<String> {
        match self {
            RunEvent::Completed { output, .. } => Some(output.clone()),
            RunEvent::Stopped(why) => Some(why.clone()),
            RunEvent::Failed(error) => Some(format!("failed: {error}")),
            RunEvent::StepStarted(_)
            | RunEvent::Text(_)
            | RunEvent::ToolCalled { .. }
            | RunEvent::Note(_)
            | RunEvent::StepResult(_)
            | RunEvent::StepFinished(_) => None,
        }
    }
}

impl ChainLog {
    pub fn start(&mut self, chain: &[String], task: &str) {
        self.title = chain.join(ARROW);
        self.running = true;
        self.push(Kind::Run, format!("{} · {task}", self.title));
    }

    pub fn receive(&mut self, event: RunEvent) {
        match event {
            RunEvent::StepStarted(agent) => self.push(Kind::Step, format!("→ {agent}")),
            RunEvent::Text(text) => self.say(&text),
            RunEvent::ToolCalled {
                call,
                refusal: None,
            } => self.push(Kind::Called, format!("· {call}")),
            RunEvent::ToolCalled {
                call,
                refusal: Some(refusal),
            } => self.push(Kind::Refused, format!("✗ {call} — {refusal}")),
            RunEvent::Note(note) => self.push(Kind::Note, note),
            RunEvent::StepResult(result) => self.push(Kind::Result, result),
            RunEvent::StepFinished(summary) => self.push(Kind::Summary, summary),
            RunEvent::Completed { total, .. } => self.finish(Kind::Summary, total),
            RunEvent::Stopped(why) => self.finish(Kind::Problem, why),
            RunEvent::Failed(error) => self.finish(Kind::Problem, format!("failed: {error}")),
        }
    }

    pub fn lines(&self) -> &[LogLine] {
        &self.lines
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    fn say(&mut self, text: &str) {
        match self.lines.last_mut() {
            Some(line) if line.kind == Kind::Said => line.text.push_str(text),
            Some(_) | None => self.push(Kind::Said, text.to_owned()),
        }
    }

    fn finish(&mut self, kind: Kind, text: String) {
        self.running = false;
        self.push(kind, text);
    }

    fn push(&mut self, kind: Kind, text: String) {
        self.lines.push(LogLine { kind, text });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started() -> ChainLog {
        let mut log = ChainLog::default();
        log.start(&["explorer".to_owned(), "reviewer".to_owned()], "check it");
        log
    }

    fn shown(log: &ChainLog) -> Vec<(Kind, &str)> {
        log.lines()
            .iter()
            .map(|line| (line.kind, line.text.as_str()))
            .collect()
    }

    fn called(call: &str, refusal: Option<&str>) -> RunEvent {
        RunEvent::ToolCalled {
            call: call.to_owned(),
            refusal: refusal.map(str::to_owned),
        }
    }

    #[test]
    fn new_log_is_empty_and_idle() {
        let log = ChainLog::default();
        assert!(log.lines().is_empty());
        assert!(!log.is_running());
    }

    #[test]
    fn start_names_the_chain_and_the_task() {
        let log = started();
        assert_eq!(shown(&log), [(Kind::Run, "explorer → reviewer · check it")]);
        assert_eq!(log.title(), "explorer → reviewer");
        assert!(log.is_running());
    }

    #[test]
    fn step_events_become_lines_in_order() {
        let mut log = started();
        log.receive(RunEvent::StepStarted("explorer".to_owned()));
        log.receive(called("grep {}", None));
        log.receive(called("read_file {}", Some("not given")));
        log.receive(RunEvent::Note("reminding the agent".to_owned()));
        log.receive(RunEvent::StepFinished("explorer · turns: 2".to_owned()));
        log.receive(RunEvent::StepResult("{}".to_owned()));
        assert_eq!(
            shown(&log)[1..],
            [
                (Kind::Step, "→ explorer"),
                (Kind::Called, "· grep {}"),
                (Kind::Refused, "✗ read_file {} — not given"),
                (Kind::Note, "reminding the agent"),
                (Kind::Summary, "explorer · turns: 2"),
                (Kind::Result, "{}"),
            ]
        );
    }

    #[test]
    fn streamed_text_is_joined_into_one_line_until_something_else_happens() {
        let mut log = started();
        log.receive(RunEvent::Text("Let me ".to_owned()));
        log.receive(RunEvent::Text("look.".to_owned()));
        log.receive(called("grep {}", None));
        log.receive(RunEvent::Text("Done.".to_owned()));
        assert_eq!(
            shown(&log)[1..],
            [
                (Kind::Said, "Let me look."),
                (Kind::Called, "· grep {}"),
                (Kind::Said, "Done."),
            ]
        );
    }

    #[test]
    fn every_ending_stops_the_run_and_gives_a_verdict() {
        let endings = [
            (
                RunEvent::Completed {
                    output: "{}".to_owned(),
                    total: "chain · steps: 2".to_owned(),
                },
                Kind::Summary,
                "chain · steps: 2",
                "{}",
            ),
            (
                RunEvent::Stopped("stopped at `reviewer`".to_owned()),
                Kind::Problem,
                "stopped at `reviewer`",
                "stopped at `reviewer`",
            ),
            (
                RunEvent::Failed("no key".to_owned()),
                Kind::Problem,
                "failed: no key",
                "failed: no key",
            ),
        ];
        for (ending, kind, line, verdict) in endings {
            let mut log = started();
            assert_eq!(ending.verdict().as_deref(), Some(verdict));
            log.receive(ending);
            assert!(!log.is_running());
            assert_eq!(shown(&log).last(), Some(&(kind, line)));
        }
    }

    #[test]
    fn events_in_the_middle_of_a_run_give_no_verdict() {
        assert_eq!(RunEvent::StepStarted("explorer".to_owned()).verdict(), None);
        assert_eq!(RunEvent::StepResult("{}".to_owned()).verdict(), None);
    }

    #[test]
    fn second_run_is_added_below_the_first() {
        let mut log = started();
        log.receive(RunEvent::Stopped("stopped".to_owned()));
        log.start(&["planner".to_owned()], "again");
        assert_eq!(shown(&log).last(), Some(&(Kind::Run, "planner · again")));
        assert!(log.is_running());
    }
}
