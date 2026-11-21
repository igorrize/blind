use proptest::{collection::vec, prelude::*};
use serde_json::{Map, Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

use super::*;
use crate::providers::openai::test_support::{sse, text_chunk, tool_call_chunk, usage_chunk};

const SETTINGS: &str = r#"{"description":"Works.","model":"test/any-model"}"#;
const NO_FIELDS: &str = r#"{"type":"object","properties":{}}"#;
const PLAN: &str = r#"{"type":"object","required":["steps","risky"],"properties":{
    "steps":{"type":"array","maxItems":3,"items":{"type":"string","maxLength":40}},
    "risky":{"type":"boolean"}}}"#;
const FILES: &str = r#"{"type":"object","required":["files"],"properties":{
    "files":{"type":"array","maxItems":50,"items":{"type":"string","maxLength":200}}}}"#;
const REVIEW: &str = r#"{"type":"object","required":["approved"],"properties":{
    "approved":{"type":"boolean"},
    "line":{"type":"integer","minimum":1}}}"#;
const VALID_PLAN: &str = r#"{"steps":["read","write"],"risky":false}"#;
const LONGEST_STEP: usize = 40;
const MOST_STEPS: usize = 3;

fn agent(name: &str, input: &str, output: &str) -> Agent {
    let contract =
        format!(r#"{{"name":"{name}","kind":"worker","input":{input},"output":{output}}}"#);
    Agent::from_parts(&contract, SETTINGS, "Do the job.".to_owned()).expect("valid agent")
}

fn planner() -> Agent {
    agent("planner", NO_FIELDS, PLAN)
}

fn agents_with_valid_input() -> Vec<(Agent, Value)> {
    vec![
        (planner(), json!({})),
        (agent("explorer", NO_FIELDS, FILES), json!({})),
        (agent("reviewer", FILES, REVIEW), json!({"files": ["a.rs"]})),
    ]
}

fn calling(name: &str, arguments: &str) -> ToolCall {
    ToolCall {
        id: "call_1".to_owned(),
        name: name.to_owned(),
        arguments: arguments.to_owned(),
    }
}

fn turn_calling(calls: Vec<ToolCall>) -> Turn {
    Turn {
        tool_calls: calls,
        ..Turn::default()
    }
}

fn accepted(agent: &Agent, turn: &Turn) -> Option<Value> {
    match verdict(agent, turn) {
        Verdict::Accepted(output) => Some(output),
        Verdict::Rejected(_) | Verdict::NotCalled => None,
    }
}

fn follows_the_plan_contract(value: &Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    let only_declared = fields.keys().all(|name| name == "steps" || name == "risky");
    let steps = fields.get("steps").and_then(Value::as_array);
    let steps_fit =
        steps.is_some_and(|steps| steps.len() <= MOST_STEPS && steps.iter().all(is_a_short_step));
    let risky_is_boolean = fields.get("risky").is_some_and(Value::is_boolean);
    only_declared && steps_fit && risky_is_boolean
}

fn is_a_short_step(step: &Value) -> bool {
    step.as_str()
        .is_some_and(|text| text.chars().count() <= LONGEST_STEP)
}

fn plan_like() -> impl Strategy<Value = Value> {
    let step = prop_oneof![
        9 => "[a-zя ]{0,45}".prop_map(Value::from),
        1 => any::<i64>().prop_map(Value::from),
    ];
    let risky = prop_oneof![
        8 => any::<bool>().prop_map(Value::from),
        1 => Just(Value::Null),
        1 => Just(json!("no")),
    ];
    let steps = proptest::option::weighted(0.9, vec(step, 0..5));
    let risky = proptest::option::weighted(0.9, risky);
    let smuggled = proptest::bool::weighted(0.1);
    (steps, risky, smuggled).prop_map(|(steps, risky, smuggled)| {
        let mut plan = Map::new();
        plan.extend(steps.map(|steps| ("steps".to_owned(), Value::from(steps))));
        plan.extend(risky.map(|risky| ("risky".to_owned(), risky)));
        plan.extend(smuggled.then(|| ("secret".to_owned(), json!("leak"))));
        Value::Object(plan)
    })
}

fn completing(arguments: &str) -> ResponseTemplate {
    sse(&[
        tool_call_chunk("call_1", COMPLETE, arguments),
        usage_chunk(10, 5),
    ])
}

async fn serving_in_order(responses: Vec<ResponseTemplate>) -> MockServer {
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

async fn serving_always(response: ResponseTemplate) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(response)
        .mount(&server)
        .await;
    server
}

async fn run(server: &MockServer, agent: &Agent, input: &Value) -> StepOutcome {
    let client = OpenAiCompatible::new(&server.uri(), None);
    run_step(None, &client, agent, "ship it", input, &mut |_| {})
        .await
        .expect("provider answers")
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    let requests = server.received_requests().await.expect("recorded");
    requests
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("json body"))
        .collect()
}

proptest! {
    #[test]
    fn result_is_accepted_exactly_when_it_follows_the_contract(plan in plan_like()) {
        let turn = turn_calling(vec![calling(COMPLETE, &plan.to_string())]);
        let output = accepted(&planner(), &turn);
        prop_assert_eq!(output.is_some(), follows_the_plan_contract(&plan));
    }

    #[test]
    fn accepted_result_is_exactly_what_the_model_sent(plan in plan_like()) {
        let turn = turn_calling(vec![calling(COMPLETE, &plan.to_string())]);
        if let Some(output) = accepted(&planner(), &turn) {
            prop_assert_eq!(output, plan);
        }
    }

    #[test]
    fn arbitrary_text_as_arguments_is_never_accepted_unless_it_is_a_valid_result(
        arguments in ".{0,60}"
    ) {
        let turn = turn_calling(vec![calling(COMPLETE, &arguments)]);
        if let Some(output) = accepted(&planner(), &turn) {
            prop_assert!(follows_the_plan_contract(&output));
        }
    }

    #[test]
    fn no_other_tool_name_can_finish_the_step(name in "[a-z_]{1,12}") {
        prop_assume!(name != COMPLETE);
        let turn = turn_calling(vec![calling(&name, VALID_PLAN)]);
        prop_assert!(matches!(verdict(&planner(), &turn), Verdict::NotCalled));
    }

    #[test]
    fn text_alone_never_finishes_the_step(text in ".{0,200}") {
        let turn = Turn { text, ..Turn::default() };
        prop_assert!(matches!(verdict(&planner(), &turn), Verdict::NotCalled));
    }
}

#[test]
fn first_complete_call_decides_when_the_model_sends_two() {
    let broken_then_valid = turn_calling(vec![
        calling(COMPLETE, r#"{"steps":[]}"#),
        calling(COMPLETE, VALID_PLAN),
    ]);
    assert!(accepted(&planner(), &broken_then_valid).is_none());
}

#[tokio::test]
async fn every_agent_is_offered_only_complete_built_from_its_own_contract() {
    for (agent, input) in agents_with_valid_input() {
        let server = serving_always(sse(&[text_chunk("thinking")])).await;
        run(&server, &agent, &input).await;
        let bodies = request_bodies(&server).await;
        let tools = bodies[0]["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 1, "{}", agent.contract().name());
        assert_eq!(tools[0]["function"]["name"], COMPLETE);
        assert_eq!(
            tools[0]["function"]["parameters"],
            agent.contract().output().to_json_schema()
        );
    }
}

#[tokio::test]
async fn step_never_makes_more_requests_than_the_turn_limit() {
    for stubborn in [
        sse(&[text_chunk("I will just talk."), usage_chunk(10, 5)]),
        completing(r#"{"steps":"not a list"}"#),
        completing("{broken json"),
        sse(&[tool_call_chunk("call_1", "bash", "{}")]),
    ] {
        let server = serving_always(stubborn).await;
        let outcome = run(&server, &planner(), &json!({})).await;
        assert!(matches!(outcome, StepOutcome::Failed { .. }), "{outcome:?}");
        let requests = server.received_requests().await.expect("recorded");
        assert_eq!(
            requests.len(),
            usize::try_from(MAX_FAILED_TURNS).expect("small")
        );
    }
}

#[tokio::test]
async fn call_to_any_other_tool_is_blocked_and_the_model_is_told() {
    let reaching_for_shell = sse(&[tool_call_chunk("call_1", "bash", r#"{"command":"ls"}"#)]);
    let server = serving_in_order(vec![reaching_for_shell, completing(VALID_PLAN)]).await;
    let outcome = run(&server, &planner(), &json!({})).await;
    assert!(
        matches!(outcome, StepOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let bodies = request_bodies(&server).await;
    let second_request = bodies[1].to_string();
    assert!(
        second_request.contains("blocked: tool `bash` does not exist"),
        "{second_request}"
    );
}

#[tokio::test]
async fn invalid_input_makes_no_request_for_any_agent() {
    for (agent, _) in agents_with_valid_input() {
        let server = serving_always(completing(VALID_PLAN)).await;
        let outcome = run(&server, &agent, &json!({"undeclared": true})).await;
        assert!(
            matches!(outcome, StepOutcome::InputRejected(_)),
            "{outcome:?}"
        );
        let requests = server.received_requests().await.expect("recorded");
        assert!(requests.is_empty(), "{}", agent.contract().name());
    }
}

#[tokio::test]
async fn provider_failure_stops_the_step_without_a_result() {
    let server = serving_always(ResponseTemplate::new(500).set_body_string("boom")).await;
    let client = OpenAiCompatible::new(&server.uri(), None);
    let outcome = run_step(
        None,
        &client,
        &planner(),
        "ship it",
        &json!({}),
        &mut |_| {},
    )
    .await;
    assert!(matches!(
        outcome,
        Err(ProviderError::Status { status: 500, .. })
    ));
}
