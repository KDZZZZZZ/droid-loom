use std::env;

use agent_smoke::AgentCore;

const DEFAULT_API_BASE: &str = "https://api.xiaomimimo.com/v1";
const DEFAULT_MODEL: &str = "mimo-v2.5-pro";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let prompt = if prompt.trim().is_empty() {
        "Say hello in one short sentence.".to_string()
    } else {
        prompt
    };

    let api_key = env_first(&["MIMO_API_KEY", "DEEPSEEK_API_KEY"])
        .ok_or("MIMO_API_KEY is not set. Set it before running this demo.")?;
    let model =
        env_first(&["MIMO_MODEL", "DEEPSEEK_MODEL"]).unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let api_base = env_first(&["MIMO_API_BASE", "DEEPSEEK_API_BASE"])
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string());

    let agent = AgentCore::new_with_base_url(api_base, api_key, model);

    println!("model: {}", agent.model());
    println!("user: {prompt}");

    let response = agent.prompt(prompt)?;
    println!("assistant: {}", response.answer);
    println!("message_count: {}", response.message_count);

    Ok(())
}

fn env_first(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| env::var(key).ok().filter(|value| !value.trim().is_empty()))
}
