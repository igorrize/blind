use crate::agent_name::AgentName;
use crate::contract::Contract;
use crate::schema::Mismatch;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChainError {
    #[error("a chain needs at least one step")]
    Empty,
    #[error("`{step}` cannot go first: it needs {fields:?} from a previous step")]
    FirstStepNeedsInput {
        step: AgentName,
        fields: Vec<String>,
    },
    #[error("`{from}` cannot hand off to `{to}`: {}", describe(.mismatches))]
    Incompatible {
        from: AgentName,
        to: AgentName,
        mismatches: Vec<Mismatch>,
    },
}

pub fn check_chain(steps: &[&Contract]) -> Result<(), ChainError> {
    let first = steps.first().ok_or(ChainError::Empty)?;
    check_first_step(first)?;
    let mut handoffs = steps.iter().zip(steps.iter().skip(1));
    handoffs.try_for_each(|(from, to)| check_handoff(from, to))
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

fn check_handoff(from: &Contract, to: &Contract) -> Result<(), ChainError> {
    let mismatches = from.output().mismatches_with(to.input());
    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(ChainError::Incompatible {
            from: from.name().clone(),
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
    const FINDINGS: &str = r#"{"type":"object","required":["findings"],"properties":{
        "findings":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":300}}}}"#;

    fn explorer() -> Contract {
        worker("explorer", NO_FIELDS, FILES)
    }

    fn reviewer() -> Contract {
        worker("reviewer", FILES, FINDINGS)
    }

    fn fixer() -> Contract {
        worker("fixer", FINDINGS, NO_FIELDS)
    }

    #[test]
    fn empty_chain_is_rejected() {
        assert_eq!(check_chain(&[]), Err(ChainError::Empty));
    }

    #[test]
    fn single_step_that_needs_only_the_task_is_a_chain() {
        assert_eq!(check_chain(&[&explorer()]), Ok(()));
    }

    #[test]
    fn steps_whose_outputs_cover_the_next_inputs_form_a_chain() {
        assert_eq!(check_chain(&[&explorer(), &reviewer(), &fixer()]), Ok(()));
    }

    #[test]
    fn step_that_needs_fields_cannot_go_first() {
        let error = check_chain(&[&reviewer(), &fixer()]).expect_err("must be rejected");
        assert_eq!(
            error.to_string(),
            r#"`reviewer` cannot go first: it needs ["files"] from a previous step"#
        );
    }

    #[test]
    fn incompatible_handoff_names_both_steps_and_the_field() {
        let error = check_chain(&[&explorer(), &fixer()]).expect_err("must be rejected");
        assert_eq!(
            error.to_string(),
            "`explorer` cannot hand off to `fixer`: `findings` is required but the previous step does not guarantee it"
        );
    }
}
