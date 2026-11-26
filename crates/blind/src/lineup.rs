use std::fmt;

const THEN: &str = " → ";
const BESIDE: &str = " + ";
const BESIDE_MARK: char = '+';

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lineup {
    steps: Vec<Vec<String>>,
}

impl Lineup {
    pub fn parse(arguments: &[String]) -> Self {
        let steps = arguments
            .iter()
            .map(|argument| argument.split(BESIDE_MARK).map(str::to_owned).collect())
            .collect();
        Self { steps }
    }

    pub fn steps(&self) -> &[Vec<String>] {
        &self.steps
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn add(&mut self, agent: String, beside: bool) {
        match self.steps.last_mut() {
            Some(last) if beside => last.push(agent),
            Some(_) | None => self.steps.push(vec![agent]),
        }
    }

    pub fn with(&self, agent: &str, beside: bool) -> Self {
        let mut next = self.clone();
        next.add(agent.to_owned(), beside);
        next
    }

    pub fn pop(&mut self) {
        let Some(last) = self.steps.last_mut() else {
            return;
        };
        last.pop();
        if last.is_empty() {
            self.steps.pop();
        }
    }
}

impl fmt::Display for Lineup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let steps: Vec<String> = self.steps.iter().map(|step| step.join(BESIDE)).collect();
        formatter.write_str(&steps.join(THEN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(arguments: &[&str]) -> Lineup {
        let arguments: Vec<String> = arguments.iter().map(|text| (*text).to_owned()).collect();
        Lineup::parse(&arguments)
    }

    #[test]
    fn each_argument_is_a_step_and_plus_joins_agents_of_one_step() {
        let lineup = parsed(&["explorer", "reviewer+planner", "coder"]);
        assert_eq!(
            lineup.steps(),
            [
                vec!["explorer".to_owned()],
                vec!["reviewer".to_owned(), "planner".to_owned()],
                vec!["coder".to_owned()]
            ]
        );
        assert_eq!(lineup.to_string(), "explorer → reviewer + planner → coder");
    }

    #[test]
    fn agent_is_added_as_the_next_step_or_beside_the_last_one() {
        let mut lineup = Lineup::default();
        lineup.add("reviewer".to_owned(), true);
        lineup.add("explorer".to_owned(), false);
        lineup.add("planner".to_owned(), true);
        assert_eq!(lineup.to_string(), "reviewer → explorer + planner");
    }

    #[test]
    fn with_leaves_the_original_untouched() {
        let lineup = parsed(&["explorer"]);
        assert_eq!(lineup.with("?", true).to_string(), "explorer + ?");
        assert_eq!(lineup.with("?", false).to_string(), "explorer → ?");
        assert_eq!(lineup.to_string(), "explorer");
    }

    #[test]
    fn pop_removes_the_last_agent_and_then_its_empty_step() {
        let mut lineup = parsed(&["explorer", "reviewer+planner"]);
        lineup.pop();
        assert_eq!(lineup.to_string(), "explorer → reviewer");
        lineup.pop();
        assert_eq!(lineup.to_string(), "explorer");
        lineup.pop();
        lineup.pop();
        assert!(lineup.is_empty());
    }
}
