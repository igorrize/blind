use blind_core::{Agent, ChainError, Contract, check_chain};

use crate::agents::Loaded;

const NEEDS_WRITING: &str = "asks for write rights, which blind cannot grant yet";

#[derive(Default)]
pub struct Crew {
    members: Vec<Agent>,
    broken: Vec<Broken>,
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

    pub fn candidates(&self, chain: &[String]) -> Vec<Candidate> {
        let before = self.contracts(chain);
        let members = self.members.iter().map(|member| candidate(member, &before));
        let (fitting, unfit): (Vec<Candidate>, Vec<Candidate>) =
            members.partition(|candidate| candidate.obstacle.is_none());
        let broken = self.broken.iter().map(unusable);
        fitting.into_iter().chain(unfit).chain(broken).collect()
    }

    pub fn pick(&self, chain: &[String], number: usize) -> Option<String> {
        let candidates = self.candidates(chain);
        let chosen = candidates.get(number.checked_sub(1)?)?;
        chosen.obstacle.is_none().then(|| chosen.name.clone())
    }

    fn contracts(&self, chain: &[String]) -> Vec<&Contract> {
        chain
            .iter()
            .filter_map(|name| self.member(name))
            .map(Agent::contract)
            .collect()
    }

    fn member(&self, name: &str) -> Option<&Agent> {
        self.members
            .iter()
            .find(|member| member.contract().name().as_str() == name)
    }
}

fn candidate(member: &Agent, before: &[&Contract]) -> Candidate {
    Candidate {
        name: member.contract().name().to_string(),
        description: member.settings().description().to_owned(),
        obstacle: obstacle(member, before),
    }
}

fn obstacle(member: &Agent, before: &[&Contract]) -> Option<String> {
    if !member.contract().rights().write().is_empty() {
        return Some(NEEDS_WRITING.to_owned());
    }
    let mut steps = before.to_vec();
    steps.push(member.contract());
    check_chain(&steps).err().map(|error| in_short(&error))
}

fn in_short(error: &ChainError) -> String {
    match error {
        ChainError::Empty => error.to_string(),
        ChainError::FirstStepNeedsInput { fields, .. } => {
            format!("cannot go first: needs {}", fields.join(", "))
        }
        ChainError::Incompatible {
            from, mismatches, ..
        } => {
            let problems: Vec<String> = mismatches.iter().map(ToString::to_string).collect();
            format!("does not fit after {from}: {}", problems.join("; "))
        }
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

    pub fn explorer_planner_reviewer() -> Crew {
        Crew::of(vec![
            loaded(agent("explorer", NO_FIELDS, FILES, "{}")),
            loaded(agent("planner", NO_FIELDS, NO_FIELDS, "{}")),
            loaded(agent("reviewer", FILES, FINDINGS, "{}")),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{FILES, NO_FIELDS, agent, broken, explorer_planner_reviewer, loaded};
    use super::*;

    fn chain(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
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
        let candidates = explorer_planner_reviewer().candidates(&[]);
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
        let after_explorer = crew.candidates(&chain(&["explorer"]));
        assert!(obstacle_of(&after_explorer, "reviewer").is_empty());
        let after_planner = crew.candidates(&chain(&["planner"]));
        let obstacle = obstacle_of(&after_planner, "reviewer");
        assert!(
            obstacle.starts_with("does not fit after planner: "),
            "{obstacle}"
        );
    }

    #[test]
    fn agent_that_wants_to_write_cannot_be_put_in_a_chain_yet() {
        let fixer = agent(
            "fixer",
            NO_FIELDS,
            NO_FIELDS,
            r#"{"read":["**"],"write":["src/**"]}"#,
        );
        let candidates = Crew::of(vec![loaded(fixer)]).candidates(&[]);
        assert_eq!(obstacle_of(&candidates, "fixer"), NEEDS_WRITING);
    }

    #[test]
    fn broken_agent_is_listed_last_with_the_reason() {
        let crew = Crew::of(vec![
            broken("ghost"),
            loaded(agent("explorer", NO_FIELDS, FILES, "{}")),
        ]);
        let candidates = crew.candidates(&[]);
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
        assert_eq!(crew.pick(&[], 1), Some("explorer".to_owned()));
        assert_eq!(crew.pick(&[], 2), Some("planner".to_owned()));
        assert_eq!(crew.pick(&[], 3), None);
        assert_eq!(crew.pick(&[], 0), None);
        assert_eq!(crew.pick(&[], 9), None);
    }

    #[test]
    fn crew_without_agents_offers_nothing() {
        assert!(Crew::default().candidates(&[]).is_empty());
    }
}
