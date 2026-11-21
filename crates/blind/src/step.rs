use std::{fmt, path::Path};

use anyhow::bail;
use blind_core::{Agent, Limits, Usage, Violation};
use blind_exec::Workspace;
use serde_json::Value;

use crate::providers::{Message, OpenAiCompatible, ProviderError, ToolCall, ToolSpec, Turn};

#[cfg(test)]
mod files;
#[cfg(test)]
mod finishing;
#[cfg(test)]
mod guarantees;

pub const MAX_FAILED_TURNS: u32 = 4;
pub const FINISHING_TURNS: u32 = 2;

const COMPLETE: &str = "complete";
const COMPLETE_DESCRIPTION: &str =
    "Finish the job. Call it exactly once, with the result that follows the schema.";
const PREAMBLE: &str = "You are a worker agent in the blind harness. Do the one job described below. \
Finish by calling the `complete` tool with the result. Do not answer in plain text: \
only a `complete` call is accepted.";
const FILES_NOTE: &str = "You can look at the project with the other tools. \
Base the result on what you actually read, do not guess file names or contents.";
const REMINDER: &str = "You have not finished. Call the `complete` tool with the result now.";
const FINISH_NOW: &str = "You may not look at the project any more. \
Call the `complete` tool now with the result based on what you have already seen. \
Do not guess what you have not seen.";
const NOT_RUN: &str = "not run: `complete` was called in the same turn";
const NO_MORE_CALLS: &str = "not run: you may not call tools any more, call `complete` now";

#[derive(Debug, PartialEq)]
pub enum StepOutcome {
    Completed { output: Value, report: StepReport },
    InputRejected(Vec<Violation>),
    Failed { report: StepReport, stop: Stop },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StepReport {
    pub usage: Usage,
    pub turns: u32,
    pub rejections: u32,
    pub tool_calls: u32,
    pub forced_to_finish: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    OutOfTokens,
    OutOfTurns,
    NoValidResult,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Progress<'a> {
    Text(&'a str),
    ToolCalled {
        name: &'a str,
        arguments: &'a str,
        refusal: Option<&'a str>,
    },
    Rejected(&'a str),
    Reminded,
    ToldToFinish(&'a str),
}

enum Verdict {
    Accepted(Value),
    Rejected(String),
    NotCalled,
}

impl StepReport {
    fn tokens(&self) -> u64 {
        self.usage.input_tokens + self.usage.output_tokens
    }

    fn stop_under(&self, limits: Limits, failed_turns: u32) -> Option<Stop> {
        if self.tokens() >= limits.tokens {
            Some(Stop::OutOfTokens)
        } else if self.turns >= limits.turns {
            Some(Stop::OutOfTurns)
        } else if failed_turns >= MAX_FAILED_TURNS {
            Some(Stop::NoValidResult)
        } else {
            None
        }
    }

    fn reason_to_finish(&self, limits: Limits) -> Option<&'static str> {
        if self.tool_calls >= limits.tool_calls {
            Some("the tool call limit is reached")
        } else if self.tokens() * 3 >= limits.tokens * 2 {
            Some("two thirds of the token budget are spent")
        } else if self.turns + FINISHING_TURNS >= limits.turns {
            Some("only the last turns are left")
        } else {
            None
        }
    }
}

impl fmt::Display for StepReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "turns: {}, tool calls: {}, rejected: {} · tokens: {} in, {} out",
            self.turns,
            self.tool_calls,
            self.rejections,
            self.usage.input_tokens,
            self.usage.output_tokens
        )?;
        if self.forced_to_finish {
            write!(formatter, " · forced to finish")?;
        }
        Ok(())
    }
}

impl Stop {
    pub fn explained(self) -> &'static str {
        match self {
            Stop::OutOfTokens => "it used more tokens than one step may",
            Stop::OutOfTurns => "it used every turn without finishing",
            Stop::NoValidResult => "it gave no valid result",
        }
    }
}

pub fn workspace_for(agent: &Agent, project: &Path) -> anyhow::Result<Option<Workspace>> {
    let rights = agent.contract().rights();
    if !rights.write().is_empty() {
        bail!(
            "agent `{}` asks for write rights, which blind cannot grant yet",
            agent.contract().name()
        );
    }
    if rights.read().is_empty() {
        return Ok(None);
    }
    let workspace = Workspace::open(project, rights.read(), &rights.tools())?;
    Ok(Some(workspace))
}

pub async fn run_step(
    workspace: Option<&Workspace>,
    client: &OpenAiCompatible,
    agent: &Agent,
    task: &str,
    input: &Value,
    on_progress: &mut impl FnMut(Progress<'_>),
) -> Result<StepOutcome, ProviderError> {
    let rejected = agent.contract().input().violations(input);
    if !rejected.is_empty() {
        return Ok(StepOutcome::InputRejected(rejected));
    }
    let model = agent.settings().model().model();
    let limits = agent.settings().limits();
    let every_tool = offered_tools(agent, workspace);
    let only_complete = offered_tools(agent, None);
    let given = given_files(agent, input);
    let mut messages = vec![
        Message::system(&system_prompt(agent, workspace)),
        Message::user(&assignment(task, input)),
    ];
    let mut report = StepReport::default();
    let mut failed_turns = 0;
    loop {
        if let Some(stop) = report.stop_under(limits, failed_turns) {
            return Ok(StepOutcome::Failed { report, stop });
        }
        if workspace.is_some()
            && !report.forced_to_finish
            && let Some(reason) = report.reason_to_finish(limits)
        {
            report.forced_to_finish = true;
            on_progress(Progress::ToldToFinish(reason));
            messages.push(Message::user(FINISH_NOW));
        }
        let (tools, calls_left) = if report.forced_to_finish {
            (&only_complete, 0)
        } else {
            (&every_tool, limits.tool_calls - report.tool_calls)
        };
        let show_text = |text: &str| on_progress(Progress::Text(text));
        let turn = client.turn(model, &messages, tools, show_text).await?;
        report.turns += 1;
        report.usage = report.usage.plus(turn.usage);
        messages.push(Message::assistant_turn(&turn));
        match verdict(agent, &turn) {
            Verdict::Accepted(output) => return Ok(StepOutcome::Completed { output, report }),
            Verdict::Rejected(reason) => {
                failed_turns += 1;
                report.rejections += 1;
                on_progress(Progress::Rejected(&reason));
                messages.extend(replies_to_rejected(&turn, &reason));
            }
            Verdict::NotCalled => {
                let reach = Reach {
                    workspace,
                    given: given.as_deref(),
                    calls_left,
                };
                let worked = work(&turn, &reach, &mut messages, on_progress);
                report.tool_calls += worked;
                if worked == 0 {
                    failed_turns += 1;
                    on_progress(Progress::Reminded);
                    messages.push(Message::user(REMINDER));
                }
            }
        }
    }
}

struct Reach<'a> {
    workspace: Option<&'a Workspace>,
    given: Option<&'a [String]>,
    calls_left: u32,
}

fn offered_tools(agent: &Agent, workspace: Option<&Workspace>) -> Vec<ToolSpec> {
    let complete = ToolSpec {
        name: COMPLETE,
        description: COMPLETE_DESCRIPTION,
        parameters: agent.contract().output().to_json_schema(),
    };
    let reading = workspace.map(Workspace::offered).unwrap_or_default();
    let reading = reading.into_iter().map(|tool| ToolSpec {
        name: tool.name,
        description: tool.description,
        parameters: tool.parameters,
    });
    std::iter::once(complete).chain(reading).collect()
}

fn system_prompt(agent: &Agent, workspace: Option<&Workspace>) -> String {
    let prompt = agent.prompt().trim();
    match workspace {
        Some(_) => format!("{PREAMBLE}\n{FILES_NOTE}\n\n{prompt}"),
        None => format!("{PREAMBLE}\n\n{prompt}"),
    }
}

fn assignment(task: &str, input: &Value) -> String {
    format!("Task:\n{task}\n\nInput:\n{input:#}")
}

fn given_files(agent: &Agent, input: &Value) -> Option<Vec<String>> {
    let field = agent.contract().rights().read_from()?;
    let listed = input[field].as_array().into_iter().flatten();
    Some(
        listed
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

fn verdict(agent: &Agent, turn: &Turn) -> Verdict {
    let Some(call) = turn.tool_calls.iter().find(|call| call.name == COMPLETE) else {
        return Verdict::NotCalled;
    };
    let output: Value = match serde_json::from_str(&call.arguments) {
        Ok(output) => output,
        Err(error) => return Verdict::Rejected(rejection(&[format!("not valid JSON: {error}")])),
    };
    let violations = agent.contract().output().violations(&output);
    if violations.is_empty() {
        Verdict::Accepted(output)
    } else {
        let problems: Vec<String> = violations.iter().map(Violation::to_string).collect();
        Verdict::Rejected(rejection(&problems))
    }
}

fn rejection(problems: &[String]) -> String {
    format!(
        "rejected: {}. Fix the result and call `complete` again.",
        problems.join("; ")
    )
}

fn replies_to_rejected(turn: &Turn, reason: &str) -> Vec<Message> {
    turn.tool_calls
        .iter()
        .map(|call| {
            let reply = if call.name == COMPLETE {
                reason
            } else {
                NOT_RUN
            };
            Message::tool_result(call, reply)
        })
        .collect()
}

fn work(
    turn: &Turn,
    reach: &Reach<'_>,
    messages: &mut Vec<Message>,
    on_progress: &mut impl FnMut(Progress<'_>),
) -> u32 {
    let mut worked = 0;
    for call in &turn.tool_calls {
        let reply = if worked >= reach.calls_left {
            NO_MORE_CALLS.to_owned()
        } else {
            match answered(call, reach) {
                Some(outcome) => {
                    worked += 1;
                    on_progress(Progress::ToolCalled {
                        name: &call.name,
                        arguments: &call.arguments,
                        refusal: outcome.as_ref().err().map(String::as_str),
                    });
                    outcome.unwrap_or_else(|refusal| format!("error: {refusal}"))
                }
                None => blocked(call),
            }
        };
        messages.push(Message::tool_result(call, &reply));
    }
    worked
}

fn answered(call: &ToolCall, reach: &Reach<'_>) -> Option<Result<String, String>> {
    let result = reach
        .workspace?
        .call(&call.name, &call.arguments, reach.given)?;
    Some(result.map_err(|refusal| refusal.to_string()))
}

fn blocked(call: &ToolCall) -> String {
    format!("blocked: tool `{}` does not exist", call.name)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, body_string_contains, method},
    };

    use super::*;
    use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};

    const CONTRACT: &str = r#"{"name":"planner","kind":"worker",
        "input":{"type":"object","properties":{"focus":{"type":"string"}}},
        "output":{"type":"object","required":["steps","risky"],"properties":{
            "steps":{"type":"array","maxItems":3,"items":{"type":"string","maxLength":40}},
            "risky":{"type":"boolean"}}}}"#;
    const SETTINGS: &str = r#"{"description":"Plans.","model":"test/planner-model"}"#;
    const VALID: &str = r#"{"steps":["read","write"],"risky":false}"#;

    fn planner() -> Agent {
        Agent::from_parts(CONTRACT, SETTINGS, "Split the task into steps.".to_owned())
            .expect("valid agent")
    }

    fn completing(arguments: &str) -> ResponseTemplate {
        sse(&[
            tool_call_chunk("call_1", COMPLETE, arguments),
            usage_chunk(10, 5),
        ])
    }

    async fn serving(responses: Vec<ResponseTemplate>) -> MockServer {
        let server = MockServer::start().await;
        for response in responses {
            Mock::given(method("POST"))
                .respond_with(response)
                .up_to_n_times(1)
                .mount(&server)
                .await;
        }
        server
    }

    async fn run(server: &MockServer, input: &Value) -> (StepOutcome, Vec<String>) {
        let client = OpenAiCompatible::new(&server.uri(), None);
        let mut seen = Vec::new();
        let mut record = |progress: Progress<'_>| seen.push(format!("{progress:?}"));
        let outcome = run_step(None, &client, &planner(), "ship it", input, &mut record)
            .await
            .expect("provider answers");
        (outcome, seen)
    }

    fn report(turns: u32, rejections: u32) -> StepReport {
        let per_turn = Usage {
            input_tokens: 10,
            output_tokens: 5,
        };
        let usage = (0..turns).fold(Usage::default(), |total, _| total.plus(per_turn));
        StepReport {
            usage,
            turns,
            rejections,
            ..StepReport::default()
        }
    }

    #[tokio::test]
    async fn valid_result_completes_the_step() {
        let server = serving(vec![completing(VALID)]).await;
        let (outcome, seen) = run(&server, &json!({})).await;
        assert_eq!(
            outcome,
            StepOutcome::Completed {
                output: json!({"steps": ["read", "write"], "risky": false}),
                report: report(1, 0)
            }
        );
        assert!(seen.is_empty());
    }

    #[tokio::test]
    async fn request_carries_the_prompt_the_task_and_the_contract_as_a_tool() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({
                "model": "planner-model",
                "tools": [{"function": {"name": "complete", "parameters": {
                    "required": ["steps", "risky"], "additionalProperties": false
                }}}]
            })))
            .and(body_string_contains("Split the task into steps."))
            .and(body_string_contains("ship it"))
            .respond_with(completing(VALID))
            .expect(1)
            .mount(&server)
            .await;
        let (outcome, _) = run(&server, &json!({"focus": "tests"})).await;
        assert!(matches!(outcome, StepOutcome::Completed { .. }));
    }

    #[tokio::test]
    async fn result_that_breaks_the_contract_is_sent_back_for_a_fix() {
        let broken = r#"{"steps":["read"],"risky":"no","extra":1}"#;
        let server = serving(vec![completing(broken), completing(VALID)]).await;
        let (outcome, seen) = run(&server, &json!({})).await;
        assert_eq!(
            outcome,
            StepOutcome::Completed {
                output: json!({"steps": ["read", "write"], "risky": false}),
                report: report(2, 1)
            }
        );
        assert_eq!(seen.len(), 1);
        assert!(seen[0].contains("`risky` must be a boolean"), "{seen:?}");
        assert!(seen[0].contains("`extra` is not declared"), "{seen:?}");
    }

    #[tokio::test]
    async fn rejection_reaches_the_model_as_a_tool_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains(
                "rejected: `risky` is required but missing",
            ))
            .respond_with(completing(VALID))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(completing(r#"{"steps":[]}"#))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let (outcome, _) = run(&server, &json!({})).await;
        assert!(matches!(outcome, StepOutcome::Completed { .. }));
    }

    #[tokio::test]
    async fn plain_text_answer_gets_a_reminder() {
        let chatty = sse(&[text_chunk("Here is my plan."), usage_chunk(10, 5)]);
        let server = serving(vec![chatty, completing(VALID)]).await;
        let (outcome, seen) = run(&server, &json!({})).await;
        assert!(matches!(outcome, StepOutcome::Completed { .. }));
        assert_eq!(seen, [r#"Text("Here is my plan.")"#, "Reminded"]);
    }

    #[tokio::test]
    async fn step_fails_when_the_model_never_gives_a_valid_result() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(completing("{not json"))
            .mount(&server)
            .await;
        let (outcome, _) = run(&server, &json!({})).await;
        assert_eq!(
            outcome,
            StepOutcome::Failed {
                report: report(MAX_FAILED_TURNS, MAX_FAILED_TURNS),
                stop: Stop::NoValidResult
            }
        );
    }

    #[tokio::test]
    async fn input_that_breaks_the_contract_never_reaches_the_model() {
        let server = serving(vec![]).await;
        let (outcome, _) = run(&server, &json!({"focus": 7})).await;
        let StepOutcome::InputRejected(violations) = outcome else {
            panic!("input must be rejected");
        };
        assert_eq!(violations[0].to_string(), "`focus` must be a string");
        let requests = server.received_requests().await.expect("recorded");
        assert!(requests.is_empty());
    }

    #[test]
    fn report_line_marks_a_forced_finish() {
        let forced = StepReport {
            forced_to_finish: true,
            ..report(2, 1)
        };
        assert_eq!(
            report(2, 1).to_string(),
            "turns: 2, tool calls: 0, rejected: 1 · tokens: 20 in, 10 out"
        );
        assert_eq!(
            forced.to_string(),
            "turns: 2, tool calls: 0, rejected: 1 · tokens: 20 in, 10 out · forced to finish"
        );
    }

    #[test]
    fn every_stop_is_explained_differently() {
        assert_eq!(
            Stop::OutOfTokens.explained(),
            "it used more tokens than one step may"
        );
        assert_eq!(
            Stop::OutOfTurns.explained(),
            "it used every turn without finishing"
        );
        assert_eq!(Stop::NoValidResult.explained(), "it gave no valid result");
    }
}
