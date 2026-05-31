use agent_core::agent_definition::AgentDefinitionBuilder;
use agent_core::context::{ContextBuildInput, ContextBuilder};
use agent_core::llm_deepseek_chat::{
    DeepSeekChatProvider, DEEPSEEK_API_KEY_ENV, DEEPSEEK_DEFAULT_ENDPOINT,
};
use agent_core::llm_provider::LlmProvider;
use agent_core::llm_stream::LlmStreamEvent;
use agent_core::run_message::RunMessage;
use agent_core::tool_schema::ToolSchema;
use agent_core::{AgentCoreError, AgentCoreResult, ContentBlock};
use serde_json::json;

fn main() -> AgentCoreResult<()> {
    let definition = AgentDefinitionBuilder::new()
        .name("deepseek_demo")
        .system_prompt("You are a concise assistant.")
        .build()?;

    let mut input = ContextBuildInput::new("deepseek-v4-flash");
    input
        .run_messages
        .push(RunMessage::user(vec![ContentBlock::text(
            "用一句话解释 agent graph。",
        )])?);
    input.visible_tool_schemas.push(ToolSchema::new(
        "lookup_notes",
        "Search local notes",
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" }
            },
            "additionalProperties": false
        }),
    )?);
    input.metadata.insert(
        "deepseek.thinking".to_string(),
        json!({ "type": "disabled" }),
    );

    let request = ContextBuilder::new().build(&definition, input)?;
    let provider = match std::env::var(DEEPSEEK_API_KEY_ENV) {
        Ok(api_key) => DeepSeekChatProvider::new().with_api_key(api_key),
        Err(_) => DeepSeekChatProvider::new(),
    };
    let prepared = provider.prepare_request(&request)?;

    println!("provider={}", prepared.provider);
    println!("endpoint={}", prepared.endpoint);
    println!("model={}", prepared.body["model"]);
    println!(
        "messages={}",
        prepared
            .body
            .get("messages")
            .and_then(|value| value.as_array())
            .map(Vec::len)
            .unwrap_or_default()
    );
    println!(
        "tools={}",
        prepared
            .body
            .get("tools")
            .and_then(|value| value.as_array())
            .map(Vec::len)
            .unwrap_or_default()
    );
    println!(
        "auth_header_present={}",
        prepared.headers.contains_key("Authorization")
    );

    if prepared.headers.contains_key("Authorization")
        && prepared.endpoint.as_str() == DEEPSEEK_DEFAULT_ENDPOINT
    {
        let response = ureq::post(&prepared.endpoint)
            .set(
                "Authorization",
                prepared
                    .headers
                    .get("Authorization")
                    .expect("authorization header is present"),
            )
            .set("Content-Type", "application/json")
            .send_json(prepared.body)
            .map_err(|error| {
                AgentCoreError::Recoverable(format!("deepseek request failed: {error}"))
            })?;
        let value: serde_json::Value = response.into_json().map_err(|error| {
            AgentCoreError::Recoverable(format!("deepseek response is not json: {error}"))
        })?;
        let events = DeepSeekChatProvider::response_events_from_value(&value)?;

        println!("live_request=true");
        println!("live_events={}", events.len());
        if let Some(text) = first_text_delta(&events) {
            println!("live_text={text}");
        }
    } else {
        println!("live_request=false");
    }

    Ok(())
}

fn first_text_delta(events: &[LlmStreamEvent]) -> Option<&str> {
    events.iter().find_map(|event| {
        if let LlmStreamEvent::TextDelta { text } = event {
            Some(text.as_str())
        } else {
            None
        }
    })
}
