use crate::app_map_memory::{
    sample_cross_app_map, AppMapMemory, CandidateAction, CandidateActionKind, LocalMapView,
};
use agent_core::agent_definition::AgentDefinition;
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_result::ToolResultStatus;
use agent_core::{AgentCoreError, AgentCoreResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossAppStepRecord {
    pub index: usize,
    pub app: String,
    pub page: String,
    pub tool: String,
    pub reused_map: bool,
    pub local_view_tokens: usize,
    pub full_map_tokens: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossAppTaskReport {
    pub completed: bool,
    pub step_count: usize,
    pub app_switches: usize,
    pub semantic_searches: usize,
    pub map_reuse_hits: usize,
    pub repeated_exploration_avoided: usize,
    pub full_map_token_total: usize,
    pub local_view_token_total: usize,
    pub token_savings_percent: f64,
    pub executed_tools: BTreeMap<String, usize>,
    pub steps: Vec<CrossAppStepRecord>,
}

impl CrossAppTaskReport {
    pub fn detail(&self) -> String {
        format!(
            "completed={}, steps={}, app_switches={}, semantic_searches={}, map_reuse_hits={}, token_savings={:.1}%",
            self.completed,
            self.step_count,
            self.app_switches,
            self.semantic_searches,
            self.map_reuse_hits,
            self.token_savings_percent
        )
    }
}

pub fn run_hundred_step_cross_app_task(
    executor: &ToolExecutor,
    definition: &AgentDefinition,
) -> AgentCoreResult<CrossAppTaskReport> {
    let map = build_long_task_map()?;
    let mut report = CrossAppTaskReport {
        completed: false,
        step_count: 0,
        app_switches: 0,
        semantic_searches: 0,
        map_reuse_hits: 0,
        repeated_exploration_avoided: 0,
        full_map_token_total: 0,
        local_view_token_total: 0,
        token_savings_percent: 0.0,
        executed_tools: BTreeMap::new(),
        steps: Vec::new(),
    };
    let mut current_page = map
        .current_page()
        .map(ToOwned::to_owned)
        .ok_or_else(|| AgentCoreError::Fatal("sample map did not set current page".to_string()))?;
    let mut current_app = page_app(&map, &current_page);

    for order_index in 1..=10 {
        report.semantic_searches += 1;
        let target = map
            .semantic_search("Share", 1)
            .into_iter()
            .next()
            .ok_or_else(|| AgentCoreError::NotFound("order detail page not in map".to_string()))?;
        let path = map
            .plan_path(&current_page, &target.page_id)
            .ok_or_else(|| {
                AgentCoreError::NotFound("path to order detail not in map".to_string())
            })?;
        report.map_reuse_hits += path.len();
        for transition in path {
            execute_action_step(
                executor,
                definition,
                &map,
                &transition.from_page,
                transition.action.clone(),
                &mut report,
            )?;
            current_page = transition.to_page;
            count_app_switch(&map, &mut current_app, &current_page, &mut report);
        }

        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(format!("observe-{order_index}"), "ui_state", json!({})),
            false,
            &mut report,
        )?;
        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(
                format!("lookup-order-{order_index}"),
                "search_database",
                json!({"query": format!("order #{:04}", 2048 + order_index)}),
            ),
            true,
            &mut report,
        )?;
        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(
                format!("remember-order-{order_index}"),
                "remember",
                json!({"information": format!("order_{order_index}=ready_to_share")}),
            ),
            true,
            &mut report,
        )?;

        current_page = execute_named_destination(
            executor,
            definition,
            &map,
            &current_page,
            "notes",
            "Open Notes",
            &mut report,
        )?;
        count_app_switch(&map, &mut current_app, &current_page, &mut report);
        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(
                format!("type-note-{order_index}"),
                "type",
                json!({"text": format!("Order {order_index}: copied detail summary"), "index": 0}),
            ),
            true,
            &mut report,
        )?;

        current_page = execute_named_destination(
            executor,
            definition,
            &map,
            &current_page,
            "calendar",
            "Open Calendar",
            &mut report,
        )?;
        count_app_switch(&map, &mut current_app, &current_page, &mut report);
        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(
                format!("type-reminder-{order_index}"),
                "type",
                json!({"text": format!("Follow up order {order_index} tomorrow"), "index": 0}),
            ),
            true,
            &mut report,
        )?;

        current_page = execute_named_destination(
            executor,
            definition,
            &map,
            &current_page,
            "mail",
            "Open Mail",
            &mut report,
        )?;
        count_app_switch(&map, &mut current_app, &current_page, &mut report);
        execute_tool_step(
            executor,
            definition,
            &map,
            &current_page,
            ToolCall::new(
                format!("type-mail-{order_index}"),
                "type",
                json!({"text": format!("Draft order {order_index} update for ops"), "index": 0}),
            ),
            true,
            &mut report,
        )?;

        current_page = execute_named_destination(
            executor,
            definition,
            &map,
            &current_page,
            "shop",
            "Open Demo Shop",
            &mut report,
        )?;
        count_app_switch(&map, &mut current_app, &current_page, &mut report);
    }

    execute_tool_step(
        executor,
        definition,
        &map,
        &current_page,
        ToolCall::new(
            "complete-hundred-step-task",
            "complete",
            json!({"success": true, "reason": "100+ step cross-app order summary workflow completed"}),
        ),
        true,
        &mut report,
    )?;
    report.completed = report.step_count >= 100;
    report.repeated_exploration_avoided = report.map_reuse_hits;
    report.token_savings_percent = if report.full_map_token_total == 0 {
        0.0
    } else {
        let saved = report
            .full_map_token_total
            .saturating_sub(report.local_view_token_total);
        saved as f64 * 100.0 / report.full_map_token_total as f64
    };
    Ok(report)
}

fn build_long_task_map() -> AgentCoreResult<AppMapMemory> {
    let mut map = sample_cross_app_map()?;
    let home = map
        .current_page()
        .map(ToOwned::to_owned)
        .ok_or_else(|| AgentCoreError::Fatal("sample map did not set current page".to_string()))?;
    let detail = map
        .semantic_search("Share", 1)
        .into_iter()
        .next()
        .ok_or_else(|| AgentCoreError::NotFound("detail page missing".to_string()))?
        .page_id;
    let notes = map
        .semantic_search("New note", 1)
        .into_iter()
        .next()
        .ok_or_else(|| AgentCoreError::NotFound("notes page missing".to_string()))?
        .page_id;
    let calendar = map
        .observe_ui_state(
            &json!({
                "package": "com.example.calendar",
                "nodes": [
                    {"text": "New event", "class": "android.widget.Button", "clickable": true},
                    {"text": "Reminder title", "class": "android.widget.EditText", "clickable": true, "editable": true}
                ]
            }),
            Some("create follow-up reminder"),
        )?
        .page_id;
    let mail = map
        .observe_ui_state(
            &json!({
                "package": "com.example.mail",
                "nodes": [
                    {"text": "Compose", "class": "android.widget.Button", "clickable": true},
                    {"text": "Message body", "class": "android.widget.EditText", "clickable": true, "editable": true}
                ]
            }),
            Some("draft operations update email"),
        )?
        .page_id;

    map.record_transition(
        notes.clone(),
        calendar.clone(),
        open_app_action(&notes, "Open Calendar", "Calendar", Some(calendar.clone())),
        true,
    );
    map.record_transition(
        calendar.clone(),
        mail.clone(),
        open_app_action(&calendar, "Open Mail", "Mail", Some(mail.clone())),
        true,
    );
    map.record_transition(
        mail.clone(),
        home.clone(),
        open_app_action(&mail, "Open Demo Shop", "Demo Shop", Some(home.clone())),
        true,
    );
    map.record_transition(
        detail.clone(),
        notes.clone(),
        open_app_action(&detail, "Open Notes", "Notes", Some(notes)),
        true,
    );
    Ok(map)
}

fn execute_named_destination(
    executor: &ToolExecutor,
    definition: &AgentDefinition,
    map: &AppMapMemory,
    from_page: &str,
    destination_query: &str,
    action_label: &str,
    report: &mut CrossAppTaskReport,
) -> AgentCoreResult<String> {
    let target = map
        .semantic_search(destination_query, 1)
        .into_iter()
        .next()
        .ok_or_else(|| {
            AgentCoreError::NotFound(format!("destination `{destination_query}` not found"))
        })?;
    let transition = map
        .transitions()
        .iter()
        .find(|transition| {
            transition.from_page == from_page
                && transition.to_page == target.page_id
                && transition.action.label == action_label
                && !transition.stale
        })
        .cloned()
        .ok_or_else(|| AgentCoreError::NotFound(format!("path for `{action_label}` not found")))?;
    report.map_reuse_hits += 1;
    execute_action_step(
        executor,
        definition,
        map,
        from_page,
        transition.action,
        report,
    )?;
    Ok(target.page_id)
}

fn execute_action_step(
    executor: &ToolExecutor,
    definition: &AgentDefinition,
    map: &AppMapMemory,
    page: &str,
    action: CandidateAction,
    report: &mut CrossAppTaskReport,
) -> AgentCoreResult<()> {
    let call = action.to_tool_call(format!("map-action-{}", report.step_count + 1));
    execute_tool_step(executor, definition, map, page, call, true, report)
}

fn execute_tool_step(
    executor: &ToolExecutor,
    definition: &AgentDefinition,
    map: &AppMapMemory,
    page: &str,
    call: ToolCall,
    reused_map: bool,
    report: &mut CrossAppTaskReport,
) -> AgentCoreResult<()> {
    let view: LocalMapView = map.local_view(Some(page), 1, None);
    let local_tokens = AppMapMemory::estimate_context_tokens(&view);
    let full_tokens = map.estimate_full_map_tokens();
    let result = executor.execute_one(definition, call.clone())?;
    if result.status != ToolResultStatus::Success {
        return Err(AgentCoreError::Recoverable(format!(
            "tool `{}` failed in long task",
            call.tool_name
        )));
    }

    report.step_count += 1;
    report.local_view_token_total += local_tokens;
    report.full_map_token_total += full_tokens;
    if reused_map {
        report.map_reuse_hits += 1;
    }
    *report
        .executed_tools
        .entry(call.tool_name.clone())
        .or_default() += 1;
    report.steps.push(CrossAppStepRecord {
        index: report.step_count,
        app: page_app(map, page),
        page: page.to_string(),
        tool: call.tool_name,
        reused_map,
        local_view_tokens: local_tokens,
        full_map_tokens: full_tokens,
    });
    Ok(())
}

fn count_app_switch(
    map: &AppMapMemory,
    current_app: &mut String,
    current_page: &str,
    report: &mut CrossAppTaskReport,
) {
    let next_app = page_app(map, current_page);
    if next_app != *current_app {
        report.app_switches += 1;
        *current_app = next_app;
    }
}

fn page_app(map: &AppMapMemory, page_id: &str) -> String {
    map.pages()
        .find(|page| page.id == page_id)
        .map(|page| page.package.clone())
        .unwrap_or_else(|| "unknown".to_string())
}

fn open_app_action(
    from_page: &str,
    label: &str,
    app_name: &str,
    expected_target: Option<String>,
) -> CandidateAction {
    CandidateAction {
        id: format!("open-app:{}", app_name.to_lowercase().replace(' ', "-")),
        label: label.to_string(),
        kind: CandidateActionKind::OpenApp,
        tool_name: "open_app".to_string(),
        arguments: json!({"text": app_name}),
        from_page: from_page.to_string(),
        expected_target,
        read_only: false,
        dangerous: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile_tools::register_mobilerun_tools;
    use crate::prompt::MobileRunLikeConfig;
    use agent_core::tool_executor::ToolExecutor;
    use agent_core::tool_registry::ToolRegistry;
    use std::sync::Arc;

    #[test]
    fn completes_hundred_step_cross_app_task_with_map_reuse() {
        let config = MobileRunLikeConfig::boundary_default();
        let definition = config.agent_definition().unwrap();
        let mut registry = ToolRegistry::new();
        register_mobilerun_tools(&mut registry).unwrap();
        let executor = ToolExecutor::new(Arc::new(registry));

        let report = run_hundred_step_cross_app_task(&executor, &definition).unwrap();

        assert!(report.completed);
        assert!(report.step_count >= 100);
        assert!(report.app_switches >= 30);
        assert!(report.map_reuse_hits >= 100);
        assert!(report.token_savings_percent > 20.0);
        assert_eq!(
            report
                .executed_tools
                .get("complete")
                .copied()
                .unwrap_or_default(),
            1
        );
    }
}
