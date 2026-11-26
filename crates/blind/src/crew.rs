use blind_core::{Agent, ChainError, Contract, check_chain, check_delivery};

use crate::agents::Loaded;
use crate::lineup::Lineup;

#[derive(Default)]
pub struct Crew {
    members: Vec<Agent>,
    broken: Vec<Broken>,
    root: Option<Contract>,
}

struct Broken {
    folder: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub description: String,
    pub obstacle: Option<String>,
}

impl Crew {
    pub fn of(loaded: Vec<Loaded>) -> Self {
        let mut crew = Self::default();
        for agent in loaded {
            match agent.outcome {
                Ok(member) => crew.members.push(member),
                Err(error) => crew.broken.push(Broken {
                    folder: agent.folder,
                    reason: error.to_string(),
                }),
            }
        }
        crew
    }

    pub fn with_root(mut self, root: Contract) -> Self {
        self.root = Some(root);
        self
    }

    pub fn delivery_problem(&self, lineup: &Lineup) -> Option<String> {
        let root = self.root.as_ref()?;
        if lineup.is_empty() {
            return None;
        }
        let delivered = check_delivery(&self.contracts(lineup), root);
        delivered.err().map(|error| error.to_string())
    }

    pub fn candidates(&self, lineup: &Lineup, beside: bool) -> Vec<Candidate> {
        let members = self
            .members
            .iter()
            .map(|member| self.candidate(member, lineup, beside));
        let (fitting, unfit): (Vec<Candidate>, Vec<Candidate>) =
            members.partition(|candidate| candidate.obstacle.is_none());
        let broken = self.broken.iter().map(unusable);
        fitting.into_iter().chain(unfit).chain(broken).collect()
    }

    pub fn pick(&self, lineup: &Lineup, beside: bool, number: usize) -> Option<String> {
        let candidates = self.candidates(lineup, beside);
        let chosen = candidates.get(number.checked_sub(1)?)?;
        chosen.obstacle.is_none().then(|| chosen.name.clone())
    }

    fn candidate(&self, member: &Agent, lineup: &Lineup, beside: bool) -> Candidate {
        Candidate {
            name: member.contract().name().to_string(),
            description: member.settings().description().to_owned(),
            obstacle: self.obstacle(member, lineup, beside),
        }
    }

    fn obstacle(&self, member: &Agent, lineup: &Lineup, beside: bool) -> Option<String> {
        let name = member.contract().name().as_str();
        let next = lineup.with(name, beside);
        check_chain(&self.contracts(&next))
            .err()
            .map(|error| in_short(&error))
    }

    fn contracts(&self, lineup: &Lineup) -> Vec<Vec<&Contract>> {
        lineup
            .steps()
            .iter()
            .map(|step| {
                let members = step.iter().filter_map(|name| self.member(name));
                members.map(Agent::contract).collect()
            })
            .collect()
    }

    fn member(&self, name: &str) -> Option<&Agent> {
        self.members
            .iter()
            .find(|member| member.contract().name().as_str() == name)
    }
}

fn in_short(error: &ChainError) -> String {
    match error {
        ChainError::FirstStepNeedsInput { fields, .. } => {
            format!("cannot go first: needs {}", fields.join(", "))
        }
        ChainError::Incompatible {
            from, mismatches, ..
        } => {
            let problems: Vec<String> = mismatches.iter().map(ToString::to_string).collect();
            format!("does not fit after {from}: {}", problems.join("; "))
        }
        ChainError::Clash { beside, clash, .. } => {
            format!("cannot run beside {beside}: {clash}")
        }
        ChainError::Empty
        | ChainError::EmptyStep
        | ChainError::TwoWriters { .. }
        | ChainError::RootCannotTake { .. } => error.to_string(),
    }
}

fn unusable(broken: &Broken) -> Candidate {
    Candidate {
        name: broken.folder.clone(),
        description: String::new(),
        obstacle: Some(format!("cannot be used: {}", broken.reason)),
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use blind_core::Agent;

    use super::Crew;
    use crate::agents::{LoadError, Loaded};

    pub const NO_FIELDS: &str = r#"{"type":"object","properties":{}}"#;
    pub const FILES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","items":{"type":"string"}}}}"#;
    pub const FINDINGS: &str = r#"{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","items":{"type":"string"}}}}"#;
    pub const SUMMARY: &str = r#"{"type":"object","required":["summary"],"properties":{
        "summary":{"type":"string"}}}"#;

    pub fn agent(name: &str, input: &str, output: &str, rights: &str) -> Agent {
        let contract = format!(
            r#"{{"name":"{name}","kind":"worker","input":{input},"output":{output},"rights":{rights}}}"#
        );
        let settings = format!(r#"{{"description":"The {name}.","model":"test/any"}}"#);
        Agent::from_parts(&contract, &settings, "Do the job.".to_owned()).expect("valid agent")
    }

    pub fn loaded(agent: Agent) -> Loaded {
        Loaded {
            folder: agent.contract().name().to_string(),
            outcome: Ok(agent),
        }
    }

    pub fn broken(folder: &str) -> Loaded {
        Loaded {
            folder: folder.to_owned(),
            outcome: Err(LoadError::NoSuchFolder(folder.to_owned())),
        }
    }

    pub fn root(input: &str) -> blind_core::Contract {
        let json = format!(r#"{{"name":"root","kind":"root","input":{input}}}"#);
        blind_core::Contract::from_json(&json).expect("valid root")
    }

    pub fn explorer_planner_reviewer() -> Crew {
        Crew::of(vec![
            loaded(agent("explorer", NO_FIELDS, FILES, "{}")),
            loaded(agent("planner", NO_FIELDS, SUMMARY, "{}")),
            loaded(agent("reviewer", FILES, FINDINGS, "{}")),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{
        FILES, NO_FIELDS, SUMMARY, agent, broken, explorer_planner_reviewer, loaded,
    };
    use super::*;

    fn lineup(arguments: &[&str]) -> Lineup {
        let arguments: Vec<String> = arguments.iter().map(|text| (*text).to_owned()).collect();
        Lineup::parse(&arguments)
    }

    fn summary(candidates: &[Candidate]) -> Vec<(String, bool)> {
        candidates
            .iter()
            .map(|candidate| (candidate.name.clone(), candidate.obstacle.is_none()))
            .collect()
    }

    fn obstacle_of(candidates: &[Candidate], name: &str) -> String {
        let candidate = candidates.iter().find(|candidate| candidate.name == name);
        candidate
            .and_then(|candidate| candidate.obstacle.clone())
            .unwrap_or_default()
    }

    #[test]
    fn agents_that_need_nothing_can_go_first_and_come_first_in_the_list() {
        let candidates = explorer_planner_reviewer().candidates(&Lineup::default(), false);
        assert_eq!(
            summary(&candidates),
            [
                ("explorer".to_owned(), true),
                ("planner".to_owned(), true),
                ("reviewer".to_owned(), false)
            ]
        );
        assert_eq!(
            obstacle_of(&candidates, "reviewer"),
            "cannot go first: needs files"
        );
    }

    #[test]
    fn next_step_fits_only_when_the_previous_result_covers_its_input() {
        let crew = explorer_planner_reviewer();
        let after_explorer = crew.candidates(&lineup(&["explorer"]), false);
        assert!(obstacle_of(&after_explorer, "reviewer").is_empty());
        let after_planner = crew.candidates(&lineup(&["planner"]), false);
        let obstacle = obstacle_of(&after_planner, "reviewer");
        assert!(
            obstacle.starts_with("does not fit after planner: "),
            "{obstacle}"
        );
    }

    #[test]
    fn agent_beside_the_last_step_must_take_what_that_step_takes() {
        let crew = explorer_planner_reviewer();
        let beside_reviewer = crew.candidates(&lineup(&["explorer", "reviewer"]), true);
        assert!(obstacle_of(&beside_reviewer, "reviewer").is_empty());
        assert_eq!(obstacle_of(&beside_reviewer, "planner"), "");
        let beside_explorer = crew.candidates(&lineup(&["explorer"]), true);
        assert_eq!(
            obstacle_of(&beside_explorer, "reviewer"),
            "cannot go first: needs files"
        );
    }

    #[test]
    fn agent_that_gives_the_same_field_of_another_kind_cannot_go_beside() {
        let crew = Crew::of(vec![
            loaded(agent("planner", NO_FIELDS, SUMMARY, "{}")),
            loaded(agent("second-planner", NO_FIELDS, SUMMARY, "{}")),
        ]);
        let candidates = crew.candidates(&lineup(&["planner"]), true);
        let obstacle = obstacle_of(&candidates, "second-planner");
        assert!(
            obstacle.starts_with("cannot run beside planner: `summary` comes from both"),
            "{obstacle}"
        );
    }

    #[test]
    fn agent_that_writes_can_be_put_in_a_chain() {
        let fixer = agent(
            "fixer",
            NO_FIELDS,
            NO_FIELDS,
            r#"{"read":["**"],"write":["src/**"]}"#,
        );
        let candidates = Crew::of(vec![loaded(fixer)]).candidates(&Lineup::default(), false);
        assert_eq!(obstacle_of(&candidates, "fixer"), "");
    }

    #[test]
    fn second_writer_cannot_go_beside_the_first() {
        let rights = r#"{"read":["**"],"write":["src/**"]}"#;
        let crew = Crew::of(vec![
            loaded(agent("fixer", NO_FIELDS, NO_FIELDS, rights)),
            loaded(agent("writer", NO_FIELDS, NO_FIELDS, rights)),
        ]);
        let candidates = crew.candidates(&lineup(&["fixer"]), true);
        let obstacle = obstacle_of(&candidates, "writer");
        assert!(obstacle.contains("both write"), "{obstacle}");
    }

    #[test]
    fn broken_agent_is_listed_last_with_the_reason() {
        let crew = Crew::of(vec![
            broken("ghost"),
            loaded(agent("explorer", NO_FIELDS, FILES, "{}")),
        ]);
        let candidates = crew.candidates(&Lineup::default(), false);
        assert_eq!(
            summary(&candidates),
            [("explorer".to_owned(), true), ("ghost".to_owned(), false)]
        );
        assert_eq!(
            obstacle_of(&candidates, "ghost"),
            "cannot be used: there is no agent folder `ghost`"
        );
    }

    #[test]
    fn only_a_fitting_candidate_can_be_picked_by_its_number() {
        let crew = explorer_planner_reviewer();
        let empty = Lineup::default();
        assert_eq!(crew.pick(&empty, false, 1), Some("explorer".to_owned()));
        assert_eq!(crew.pick(&empty, false, 2), Some("planner".to_owned()));
        assert_eq!(crew.pick(&empty, false, 3), None);
        assert_eq!(crew.pick(&empty, false, 0), None);
        assert_eq!(crew.pick(&empty, false, 9), None);
    }

    #[test]
    fn chain_whose_result_root_cannot_take_is_named_with_the_reason() {
        let wants_text = super::test_support::root(
            r#"{"type":"object","properties":{"files":{"type":"string"}}}"#,
        );
        let crew = explorer_planner_reviewer().with_root(wants_text);
        let problem = crew
            .delivery_problem(&lineup(&["explorer"]))
            .expect("a problem");
        assert!(
            problem.starts_with("root cannot take the result of `explorer`"),
            "{problem}"
        );
        assert_eq!(
            crew.delivery_problem(&lineup(&["explorer", "reviewer"])),
            None
        );
        assert_eq!(crew.delivery_problem(&Lineup::default()), None);
    }

    #[test]
    fn crew_without_agents_offers_nothing() {
        assert!(
            Crew::default()
                .candidates(&Lineup::default(), false)
                .is_empty()
        );
    }
}
