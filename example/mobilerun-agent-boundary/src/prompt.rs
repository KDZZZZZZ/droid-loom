use agent_core::agent_definition::AgentDefinitionBuilder;
use agent_core::{AgentCoreResult, AgentDefinition, ToolVisibility};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionMode {
    Fast,
    Reasoning,
}

#[derive(Clone, Debug)]
pub struct MobileRunLikeConfig {
    pub agent_name: String,
    pub mode: ExecutionMode,
    pub goal: String,
    pub max_steps: usize,
    pub step_timeout_seconds: u64,
    pub vision_enabled: bool,
    pub custom_instruction: Option<String>,
    pub variables: BTreeMap<String, String>,
    pub prompt_set: PromptSet,
}

#[derive(Clone, Debug)]
pub struct PromptSet {
    pub fast_system: String,
    pub reasoning_system: String,
    pub user_task: String,
}

impl MobileRunLikeConfig {
    pub fn boundary_default() -> Self {
        let mut variables = BTreeMap::new();
        variables.insert("app_name".to_string(), "Demo Shop".to_string());
        variables.insert("search_term".to_string(), "wireless charger".to_string());

        Self {
            agent_name: "mobilerun_like_fast_agent".to_string(),
            mode: ExecutionMode::Fast,
            goal: "Open the demo shop, search for {{search_term}}, remember the first result, then finish with a concise structured summary.".to_string(),
            max_steps: 30,
            step_timeout_seconds: 10,
            vision_enabled: true,
            custom_instruction: Some(
                "Use tools for every mobile action. Never claim a tap or type happened without a tool result."
                    .to_string(),
            ),
            variables,
            prompt_set: PromptSet {
                fast_system: FAST_SYSTEM.to_string(),
                reasoning_system: REASONING_SYSTEM.to_string(),
                user_task: USER_TASK.to_string(),
            },
        }
    }

    pub fn agent_definition(&self) -> AgentCoreResult<AgentDefinition> {
        self.agent_definition_with_tool_visibility(default_tool_visibility())
    }

    pub fn agent_definition_with_tool_visibility(
        &self,
        tool_visibility: BTreeMap<String, ToolVisibility>,
    ) -> AgentCoreResult<AgentDefinition> {
        let system_prompt = self.render_system_prompt();
        let mut builder = AgentDefinitionBuilder::new()
            .name(self.agent_name.clone())
            .system_prompt(system_prompt);

        for (tool_name, visibility) in tool_visibility {
            builder = builder.tool_visibility(tool_name, visibility);
        }

        builder.build()
    }

    pub fn render_user_prompt(&self) -> String {
        render_template(&self.prompt_set.user_task, &self.variables)
            .replace("{{goal}}", &render_template(&self.goal, &self.variables))
    }

    pub fn render_system_prompt(&self) -> String {
        let template = match self.mode {
            ExecutionMode::Fast => &self.prompt_set.fast_system,
            ExecutionMode::Reasoning => &self.prompt_set.reasoning_system,
        };
        let mut prompt = render_template(template, &self.variables)
            .replace("{{max_steps}}", &self.max_steps.to_string())
            .replace(
                "{{step_timeout_seconds}}",
                &self.step_timeout_seconds.to_string(),
            )
            .replace("{{vision_enabled}}", bool_label(self.vision_enabled));

        if let Some(custom_instruction) = &self.custom_instruction {
            prompt.push_str("\n\nCustom instruction:\n");
            prompt.push_str(custom_instruction);
        }

        prompt
    }
}

pub fn default_tool_visibility() -> BTreeMap<String, ToolVisibility> {
    BTreeMap::from([
        ("screenshot".to_string(), ToolVisibility::Direct),
        ("ui_state".to_string(), ToolVisibility::Direct),
        ("click".to_string(), ToolVisibility::Direct),
        ("click_at".to_string(), ToolVisibility::Direct),
        ("type".to_string(), ToolVisibility::Direct),
        ("swipe".to_string(), ToolVisibility::Direct),
        ("wait".to_string(), ToolVisibility::Direct),
        ("complete".to_string(), ToolVisibility::Direct),
        ("click_area".to_string(), ToolVisibility::Searchable),
        ("long_press".to_string(), ToolVisibility::Searchable),
        ("long_press_at".to_string(), ToolVisibility::Searchable),
        ("system_button".to_string(), ToolVisibility::Searchable),
        ("type_secret".to_string(), ToolVisibility::Searchable),
        ("remember".to_string(), ToolVisibility::Searchable),
        ("open_app".to_string(), ToolVisibility::Searchable),
        ("search_database".to_string(), ToolVisibility::Searchable),
        ("raw_adb_shell".to_string(), ToolVisibility::Hidden),
    ])
}

pub fn render_template(template: &str, variables: &BTreeMap<String, String>) -> String {
    let mut rendered = template.to_string();
    for (key, value) in variables {
        rendered = rendered.replace(&format!("{{{{{key}}}}}"), value);
        rendered = rendered.replace(&format!("{{{{ {key} }}}}"), value);
    }
    rendered
}

fn bool_label(value: bool) -> &'static str {
    if value {
        "enabled"
    } else {
        "disabled"
    }
}

const FAST_SYSTEM: &str = r#"You are a fast mobile automation agent.
You observe the current mobile screen, choose exactly the next useful action, and stop when the task is done.

Rules:
- Use direct tools for common actions.
- Search for uncommon tools only when needed.
- Do not use hidden tools.
- Keep at most {{max_steps}} steps.
- Each tool action should finish within {{step_timeout_seconds}} seconds.
- Vision is {{vision_enabled}}.
- Return a final JSON object with success, remembered_items, and summary."#;

const REASONING_SYSTEM: &str = r#"You are a reasoning mobile automation agent.
Plan briefly, delegate concrete mobile actions to tool calls, inspect observations, and finish with a structured answer.

Rules:
- Keep the plan short.
- Prefer screen state and tool observations over assumptions.
- Keep at most {{max_steps}} steps.
- Vision is {{vision_enabled}}."#;

const USER_TASK: &str = r#"Task:
{{goal}}

Target app: {{app_name}}"#;
