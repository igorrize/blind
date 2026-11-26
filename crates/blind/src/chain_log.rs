use crate::lineup::Lineup;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunEvent {
    StepStarted(Vec<String>),
    Step {
        agent: String,
        kind: Kind,
        text: String,
    },
    StepResult(String),
    StepFinished(String),
    Changes(String),
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
    side_by_side: bool,
    speaker: Option<String>,
    changes: Option<String>,
}

impl RunEvent {
    pub fn verdict(&self) -> Option<String> {
        match self {
            RunEvent::Completed { output, .. } => Some(output.clone()),
            RunEvent::Stopped(why) => Some(why.clone()),
            RunEvent::Failed(error) => Some(format!("failed: {error}")),
            RunEvent::StepStarted(_)
            | RunEvent::Step { .. }
            | RunEvent::StepResult(_)
            | RunEvent::StepFinished(_)
            | RunEvent::Changes(_) => None,
        }
    }
}

impl ChainLog {
    pub fn start(&mut self, lineup: &Lineup, task: &str) {
        self.title = lineup.to_string();
        self.running = true;
        self.side_by_side = false;
        self.speaker = None;
        self.changes = None;
        self.push(Kind::Run, format!("{} · {task}", self.title));
    }

    pub fn receive(&mut self, event: RunEvent) {
        match event {
            RunEvent::StepStarted(agents) => {
                self.side_by_side = agents.len() > 1;
                self.push(Kind::Step, format!("→ {}", agents.join(" + ")));
            }
            RunEvent::Step {
                agent,
                kind: Kind::Said,
                text,
            } => self.say(&agent, &text),
            RunEvent::Step { agent, kind, text } => {
                let line = self.signed(&agent, &text);
                self.push(kind, line);
            }
            RunEvent::StepResult(result) => self.push(Kind::Result, result),
            RunEvent::StepFinished(summary) => self.push(Kind::Summary, summary),
            RunEvent::Changes(changes) => {
                self.push(Kind::Summary, changes.clone());
                self.changes = Some(changes);
            }
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

    pub fn changes(&self) -> Option<&str> {
        self.changes.as_deref()
    }

    fn signed(&self, agent: &str, text: &str) -> String {
        if self.side_by_side {
            format!("{agent}: {text}")
        } else {
            text.to_owned()
        }
    }

    fn say(&mut self, agent: &str, text: &str) {
        let same_speaker = self.speaker.as_deref() == Some(agent);
        match self.lines.last_mut() {
            Some(line) if line.kind == Kind::Said && same_speaker => line.text.push_str(text),
            Some(_) | None => {
                let line = self.signed(agent, text);
                self.push(Kind::Said, line);
            }
        }
        self.speaker = Some(agent.to_owned());
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

    fn started(arguments: &[&str]) -> ChainLog {
        let arguments: Vec<String> = arguments.iter().map(|text| (*text).to_owned()).collect();
        let mut log = ChainLog::default();
        log.start(&Lineup::parse(&arguments), "check it");
        log
    }

    fn shown(log: &ChainLog) -> Vec<(Kind, &str)> {
        log.lines()
            .iter()
            .map(|line| (line.kind, line.text.as_str()))
            .collect()
    }

    fn step(agents: &[&str]) -> RunEvent {
        RunEvent::StepStarted(agents.iter().map(|agent| (*agent).to_owned()).collect())
    }

    fn by(agent: &str, kind: Kind, text: &str) -> RunEvent {
        RunEvent::Step {
            agent: agent.to_owned(),
            kind,
            text: text.to_owned(),
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
        let log = started(&["explorer", "reviewer+planner"]);
        assert_eq!(
            shown(&log),
            [(Kind::Run, "explorer → reviewer + planner · check it")]
        );
        assert_eq!(log.title(), "explorer → reviewer + planner");
        assert!(log.is_running());
    }

    #[test]
    fn step_events_become_lines_in_order() {
        let mut log = started(&["explorer"]);
        log.receive(step(&["explorer"]));
        log.receive(by("explorer", Kind::Called, "· grep {}"));
        log.receive(by("explorer", Kind::Refused, "✗ read_file {} — not given"));
        log.receive(by("explorer", Kind::Note, "reminding the agent"));
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
        let mut log = started(&["explorer"]);
        log.receive(step(&["explorer"]));
        log.receive(by("explorer", Kind::Said, "Let me "));
        log.receive(by("explorer", Kind::Said, "look."));
        log.receive(by("explorer", Kind::Called, "· grep {}"));
        log.receive(by("explorer", Kind::Said, "Done."));
        assert_eq!(
            shown(&log)[2..],
            [
                (Kind::Said, "Let me look."),
                (Kind::Called, "· grep {}"),
                (Kind::Said, "Done."),
            ]
        );
    }

    #[test]
    fn agents_working_side_by_side_sign_their_lines_and_never_share_one() {
        let mut log = started(&["reviewer+planner"]);
        log.receive(step(&["reviewer", "planner"]));
        log.receive(by("reviewer", Kind::Said, "Read"));
        log.receive(by("planner", Kind::Said, "Plan"));
        log.receive(by("reviewer", Kind::Said, "ing."));
        log.receive(by("planner", Kind::Called, "· grep {}"));
        assert_eq!(
            shown(&log)[1..],
            [
                (Kind::Step, "→ reviewer + planner"),
                (Kind::Said, "reviewer: Read"),
                (Kind::Said, "planner: Plan"),
                (Kind::Said, "reviewer: ing."),
                (Kind::Called, "planner: · grep {}"),
            ]
        );
    }

    #[test]
    fn single_agent_after_a_group_signs_nothing_again() {
        let mut log = started(&["reviewer+planner", "coder"]);
        log.receive(step(&["reviewer", "planner"]));
        log.receive(step(&["coder"]));
        log.receive(by("coder", Kind::Called, "· grep {}"));
        assert_eq!(shown(&log).last(), Some(&(Kind::Called, "· grep {}")));
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
            let mut log = started(&["explorer"]);
            assert_eq!(ending.verdict().as_deref(), Some(verdict));
            log.receive(ending);
            assert!(!log.is_running());
            assert_eq!(shown(&log).last(), Some(&(kind, line)));
        }
    }

    #[test]
    fn events_in_the_middle_of_a_run_give_no_verdict() {
        assert_eq!(step(&["explorer"]).verdict(), None);
        assert_eq!(RunEvent::StepResult("{}".to_owned()).verdict(), None);
    }

    #[test]
    fn changes_of_a_run_are_shown_kept_and_forgotten_by_the_next_run() {
        let mut log = started(&["writer"]);
        log.receive(RunEvent::Changes("changed: README.md".to_owned()));
        assert_eq!(
            shown(&log).last(),
            Some(&(Kind::Summary, "changed: README.md"))
        );
        assert_eq!(log.changes(), Some("changed: README.md"));
        assert!(log.is_running());
        log.start(&Lineup::parse(&["explorer".to_owned()]), "again");
        assert_eq!(log.changes(), None);
    }

    #[test]
    fn second_run_is_added_below_the_first() {
        let mut log = started(&["explorer"]);
        log.receive(RunEvent::Stopped("stopped".to_owned()));
        log.start(&Lineup::parse(&["planner".to_owned()]), "again");
        assert_eq!(shown(&log).last(), Some(&(Kind::Run, "planner · again")));
        assert!(log.is_running());
    }
}
