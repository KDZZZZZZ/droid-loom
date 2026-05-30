use std::env;

use agent_smoke::AgentCore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let prompt = env::args().skip(1).collect::<Vec<_>>().join(" ");
    let prompt = if prompt.trim().is_empty() {
        "Say hello in one short sentence.".to_string()
    } else {
        prompt
    };

    let api_key = env::var("DEEPSEEK_API_KEY")
        .map_err(|_| "DEEPSEEK_API_KEY is not set. Set it before running this demo.")?;
    let model = env::var("DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-v4-pro".to_string());
    let api_base =
        env::var("DEEPSEEK_API_BASE").unwrap_or_else(|_| "https://api.deepseek.com".to_string());

    let agent = AgentCore::new_with_base_url(api_base, api_key, model);

    println!("model: {}", agent.model());
    println!("user: {prompt}");

    let response = agent.prompt(prompt)?;
    println!("assistant: {}", response.answer);
    println!("message_count: {}", response.message_count);

    Ok(())
}
