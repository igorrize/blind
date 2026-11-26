use std::fmt;

use crate::agent_name::AgentName;
use crate::contract::Contract;
use crate::schema::{Clash, Mismatch, ObjectSchema};

const BESIDE: &str = " + ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group(pub Vec<AgentName>);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("a chain needs at least one step")]
    Empty,
    #[error("a step needs at least one agent")]
    EmptyStep,
    #[error("`{step}` cannot go first: it needs {fields:?} from a previous step")]
    FirstStepNeedsInput {
        step: AgentName,
        fields: Vec<String>,
    },
    #[error("`{from}` cannot hand off to `{to}`: {}", describe(.mismatches))]
    Incompatible {
        from: Group,
        to: AgentName,
        mismatches: Vec<Mismatch>,
    },
    #[error("`{agent}` cannot run beside `{beside}`: {clash}")]
    Clash {
        agent: AgentName,
        beside: Group,
        clash: Clash,
    },
    #[error("root cannot take the result of `{from}`: {}", describe(.mismatches))]
    RootCannotTake {
        from: Group,
        mismatches: Vec<Mismatch>,
    },
    #[error("`{first}` and `{second}` both write; only one agent of a step may write")]
    TwoWriters { first: AgentName, second: AgentName },
}

struct Handoff {
    from: Group,
    output: ObjectSchema,
}

impl fmt::Display for Group {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.0.iter().map(AgentName::as_str).collect();
        formatter.write_str(&names.join(BESIDE))
    }
}

pub fn check_chain(steps: &[Vec<&Contract>]) -> Result<(), ChainError> {
    if steps.is_empty() {
        return Err(ChainError::Empty);
    }
    let mut previous: Option<Handoff> = None;
    for step in steps {
        let output = step_output(step)?;
        for member in step {
            match &previous {
                Some(handoff) => check_handoff(handoff, member)?,
                None => check_first_step(member)?,
            }
        }
        previous = Some(Handoff {
            from: group_of(step),
            output,
        });
    }
    Ok(())
}

pub fn check_delivery(steps: &[Vec<&Contract>], root: &Contract) -> Result<(), ChainError> {
    let Some(last) = steps.last() else {
        return Err(ChainError::Empty);
    };
    let mismatches = step_output(last)?.mismatches_with(root.input());
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(ChainError::RootCannotTake {
            from: group_of(last),
            mismatches,
        })
    }
}

fn step_output(step: &[&Contract]) -> Result<ObjectSchema, ChainError> {
    let (first, others) = step.split_first().ok_or(ChainError::EmptyStep)?;
    let mut output = first.output().clone();
    for (position, member) in others.iter().enumerate() {
        let before = &step[..=position];
        check_writers(before, member)?;
        output = output
            .merged_with(member.output())
            .map_err(|clash| ChainError::Clash {
                agent: member.name().clone(),
                beside: group_of(before),
                clash,
            })?;
    }
    Ok(output)
}

fn check_writers(before: &[&Contract], member: &Contract) -> Result<(), ChainError> {
    let earlier_writer = before.iter().find(|earlier| writes(earlier));
    match earlier_writer {
        Some(first) if writes(member) => Err(ChainError::TwoWriters {
            first: first.name().clone(),
            second: member.name().clone(),
        }),
        Some(_) | None => Ok(()),
    }
}

fn writes(contract: &Contract) -> bool {
    !contract.rights().write().is_empty()
}

fn group_of(step: &[&Contract]) -> Group {
    Group(step.iter().map(|member| member.name().clone()).collect())
}

fn check_first_step(step: &Contract) -> Result<(), ChainError> {
    let fields = step.input().required();
    if fields.is_empty() {
        Ok(())
    } else {
        Err(ChainError::FirstStepNeedsInput {
            step: step.name().clone(),
            fields: fields.to_vec(),
        })
    }
}

fn check_handoff(handoff: &Handoff, to: &Contract) -> Result<(), ChainError> {
    let mismatches = handoff.output.mismatches_with(to.input());
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(ChainError::Incompatible {
            from: handoff.from.clone(),
            to: to.name().clone(),
            mismatches,
        })
    }
}

fn describe(mismatches: &[Mismatch]) -> String {
    let lines: Vec<String> = mismatches.iter().map(Mismatch::to_string).collect();
    lines.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::test_support::{NO_FIELDS, worker};

    const FILES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}}}}"#;
    const MANY_FILES: &str = r#"{"type":"object","required":["files"],"properties":{
        "files":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":200}}}}"#;
    const FINDINGS: &str = r#"{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":300}}}}"#;
    const PLAN: &str = r#"{"type":"object","required":["steps"],"properties":{
        "steps":{"type":"array","items":{"type":"string"}}}}"#;
    const FILES_AND_PLAN: &str = r#"{"type":"object","required":["files","steps"],"properties":{
        "files":{"type":"array","items":{"type":"string","maxLength":200}},
        "steps":{"type":"array","items":{"type":"string"}}}}"#;
    const SUMMARY: &str = r#"{"type":"object","properties":{"summary":{"type":"string"}}}"#;

    struct Crew {
        explorer: Contract,
        second_explorer: Contract,
        reviewer: Contract,
        second_reviewer: Contract,
        bulk_reviewer: Contract,
        planner: Contract,
        coder: Contract,
        fixer: Contract,
    }

    fn crew() -> Crew {
        Crew {
            explorer: worker("explorer", NO_FIELDS, FILES),
            second_explorer: worker("second-explorer", NO_FIELDS, FILES),
            reviewer: worker("reviewer", FILES, FINDINGS),
            second_reviewer: worker("second-reviewer", FILES, FINDINGS),
            bulk_reviewer: worker("bulk-reviewer", MANY_FILES, FINDINGS),
            planner: worker("planner", NO_FIELDS, PLAN),
            coder: worker("coder", FILES_AND_PLAN, NO_FIELDS),
            fixer: worker("fixer", FINDINGS, NO_FIELDS),
        }
    }

    fn one_by_one<'a>(contracts: &[&'a Contract]) -> Vec<Vec<&'a Contract>> {
        contracts.iter().map(|contract| vec![*contract]).collect()
    }

    fn writer(name: &str) -> Contract {
        let json = format!(
            r#"{{"name":"{name}","kind":"worker","input":{NO_FIELDS},"output":{NO_FIELDS},
                "rights":{{"read":["**"],"write":["src/**"]}}}}"#
        );
        Contract::from_json(&json).expect("valid contract")
    }

    fn rejection(chain: &[Vec<&Contract>]) -> String {
        check_chain(chain)
            .expect_err("must be rejected")
            .to_string()
    }

    #[test]
    fn empty_chain_is_rejected() {
        assert_eq!(check_chain(&[]), Err(ChainError::Empty));
    }

    #[test]
    fn step_without_agents_is_rejected() {
        let crew = crew();
        assert_eq!(
            check_chain(&[vec![&crew.explorer], vec![]]),
            Err(ChainError::EmptyStep)
        );
    }

    #[test]
    fn single_step_that_needs_only_the_task_is_a_chain() {
        let crew = crew();
        assert_eq!(check_chain(&one_by_one(&[&crew.explorer])), Ok(()));
    }

    #[test]
    fn steps_whose_outputs_cover_the_next_inputs_form_a_chain() {
        let crew = crew();
        let chain = one_by_one(&[&crew.explorer, &crew.reviewer, &crew.fixer]);
        assert_eq!(check_chain(&chain), Ok(()));
    }

    #[test]
    fn step_that_needs_fields_cannot_go_first() {
        let crew = crew();
        assert_eq!(
            rejection(&one_by_one(&[&crew.reviewer, &crew.fixer])),
            r#"`reviewer` cannot go first: it needs ["files"] from a previous step"#
        );
    }

    #[test]
    fn incompatible_handoff_names_both_steps_and_the_field() {
        let crew = crew();
        assert_eq!(
            rejection(&one_by_one(&[&crew.explorer, &crew.fixer])),
            "`explorer` cannot hand off to `fixer`: `findings` is required but the previous step does not guarantee it"
        );
    }

    #[test]
    fn every_agent_of_a_first_step_must_need_only_the_task() {
        let crew = crew();
        let error = rejection(&[vec![&crew.explorer, &crew.reviewer]]);
        assert!(error.starts_with("`reviewer` cannot go first"), "{error}");
    }

    #[test]
    fn every_agent_of_a_step_gets_what_the_step_before_gave() {
        let crew = crew();
        let both_review = [
            vec![&crew.explorer],
            vec![&crew.reviewer, &crew.second_reviewer],
        ];
        assert_eq!(check_chain(&both_review), Ok(()));
        let one_cannot = [vec![&crew.explorer], vec![&crew.reviewer, &crew.fixer]];
        let error = rejection(&one_cannot);
        assert!(
            error.starts_with("`explorer` cannot hand off to `fixer`"),
            "{error}"
        );
    }

    #[test]
    fn next_step_gets_what_all_agents_of_a_step_gave_together() {
        let crew = crew();
        let chain = [vec![&crew.explorer, &crew.planner], vec![&crew.coder]];
        assert_eq!(check_chain(&chain), Ok(()));
        let only_files = [vec![&crew.explorer], vec![&crew.coder]];
        assert!(matches!(
            check_chain(&only_files),
            Err(ChainError::Incompatible { .. })
        ));
    }

    #[test]
    fn handoff_from_a_step_names_all_its_agents() {
        let crew = crew();
        let error = rejection(&[vec![&crew.explorer, &crew.planner], vec![&crew.fixer]]);
        assert!(
            error.starts_with("`explorer + planner` cannot hand off to `fixer`"),
            "{error}"
        );
    }

    #[test]
    fn joined_lists_add_up_and_may_no_longer_fit_the_next_step() {
        let crew = crew();
        let explorers = vec![&crew.explorer, &crew.second_explorer];
        let to_many = [explorers.clone(), vec![&crew.bulk_reviewer]];
        assert_eq!(check_chain(&to_many), Ok(()));
        let to_few = [explorers, vec![&crew.reviewer]];
        assert!(matches!(
            check_chain(&to_few),
            Err(ChainError::Incompatible { .. })
        ));
    }

    #[test]
    fn agents_that_give_the_same_field_of_another_kind_cannot_run_side_by_side() {
        let first = worker("first", NO_FIELDS, SUMMARY);
        let second = worker("second", NO_FIELDS, SUMMARY);
        assert_eq!(
            rejection(&[vec![&first, &second]]),
            "`second` cannot run beside `first`: `summary` comes from both, and only lists of the same kind can be put together"
        );
    }

    #[test]
    fn only_one_agent_of_a_step_may_write() {
        let crew = crew();
        let first = writer("first");
        let second = writer("second");
        assert_eq!(
            check_chain(&[vec![&first, &crew.explorer, &second]]),
            Err(ChainError::TwoWriters {
                first: first.name().clone(),
                second: second.name().clone()
            })
        );
        assert_eq!(check_chain(&[vec![&first, &crew.explorer]]), Ok(()));
    }

    fn root(input: &str) -> Contract {
        let json = format!(r#"{{"name":"root","kind":"root","input":{input}}}"#);
        Contract::from_json(&json).expect("valid root")
    }

    #[test]
    fn root_takes_only_the_fields_it_declares_and_ignores_the_rest() {
        let crew = crew();
        let takes_findings = root(
            r#"{"type":"object","properties":{"findings":{"type":"array","items":{"type":"string"}}}}"#,
        );
        let chain = one_by_one(&[&crew.explorer, &crew.reviewer]);
        assert_eq!(check_delivery(&chain, &takes_findings), Ok(()));
        let ends_with_files = one_by_one(&[&crew.explorer]);
        assert_eq!(check_delivery(&ends_with_files, &takes_findings), Ok(()));
    }

    #[test]
    fn root_refuses_a_result_whose_field_has_another_type() {
        let crew = crew();
        let wants_text = root(r#"{"type":"object","properties":{"files":{"type":"string"}}}"#);
        let error = check_delivery(&one_by_one(&[&crew.explorer]), &wants_text)
            .expect_err("must be refused")
            .to_string();
        assert!(
            error.starts_with("root cannot take the result of `explorer`: `files`"),
            "{error}"
        );
    }

    #[test]
    fn root_judges_the_joined_result_of_the_last_step() {
        let crew = crew();
        let few_files = root(
            r#"{"type":"object","properties":{"files":{"type":"array","maxItems":60,"items":{"type":"string","maxLength":200}}}}"#,
        );
        let one = one_by_one(&[&crew.explorer]);
        assert_eq!(check_delivery(&one, &few_files), Ok(()));
        let two = [vec![&crew.explorer, &crew.second_explorer]];
        assert!(matches!(
            check_delivery(&two, &few_files),
            Err(ChainError::RootCannotTake { .. })
        ));
    }
}
