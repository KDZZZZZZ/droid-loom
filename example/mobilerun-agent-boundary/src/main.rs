mod app_map_memory;
mod context_stability;
mod cross_app_task;
mod execution_probability;
mod key_routing;
mod mobile_tools;
mod prompt;
mod scripted_agent;
mod task_context;

fn main() -> agent_core::AgentCoreResult<()> {
    let report = scripted_agent::run_boundary_probe()?;
    report.print();
    Ok(())
}
