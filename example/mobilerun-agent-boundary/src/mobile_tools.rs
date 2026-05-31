use agent_core::tool::{Tool, ToolInvocation, ToolMetadata, ToolOutput};
use agent_core::tool_registry::ToolRegistry;
use agent_core::tool_schema::ToolSchema;
use agent_core::{AgentCoreResult, ToolVisibility};
use serde_json::{json, Map, Value};

#[derive(Debug)]
struct MobileMockTool {
    metadata: ToolMetadata,
}

impl MobileMockTool {
    fn new(schema: ToolSchema, default_visibility: ToolVisibility) -> Self {
        let mut metadata = ToolMetadata::new(schema, default_visibility);
        let name = metadata.name().to_string();
        metadata.capabilities.read_only =
            matches!(name.as_str(), "screenshot" | "ui_state" | "search_database");
        metadata.capabilities.idempotent = metadata.capabilities.read_only;
        metadata.capabilities.destructive = name == "raw_adb_shell";
        metadata.capabilities.requires_network = name == "search_database";
        metadata.execution.timeout_ms = Some(10_000);
        Self { metadata }
    }
}

impl Tool for MobileMockTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        let action = self.metadata.name();
        let arguments = redacted_arguments(action, invocation.arguments);
        let mut output = Map::new();
        output.insert("ok".to_string(), json!(true));
        output.insert("action".to_string(), json!(action));
        output.insert("call_id".to_string(), json!(invocation.call_id));
        output.insert("arguments".to_string(), arguments);

        match action {
            "screenshot" => {
                output.insert("image_uri".to_string(), json!("screen://current"));
            }
            "ui_state" => {
                output.insert(
                    "elements".to_string(),
                    json!([
                        {"id": "search_box", "text": "Search", "clickable": true},
                        {"id": "first_result", "text": "Wireless Charger Stand", "clickable": true}
                    ]),
                );
            }
            "remember" => {
                output.insert("stored".to_string(), json!(true));
            }
            "complete" => {
                output.insert("terminal".to_string(), json!(true));
            }
            "type_secret" => {
                output.insert("redacted".to_string(), json!(true));
            }
            "search_database" => {
                output.insert(
                    "matches".to_string(),
                    json!([{"id": "sku-001", "title": "Wireless Charger Stand"}]),
                );
            }
            _ => {}
        }

        Ok(ToolOutput::new(Value::Object(output)))
    }
}

pub fn register_mobilerun_tools(registry: &mut ToolRegistry) -> AgentCoreResult<()> {
    for spec in tool_specs()? {
        registry.register(MobileMockTool::new(spec.schema, spec.default_visibility))?;
    }
    Ok(())
}

pub fn expected_action_count() -> usize {
    17
}

struct ToolSpec {
    schema: ToolSchema,
    default_visibility: ToolVisibility,
}

fn tool_specs() -> AgentCoreResult<Vec<ToolSpec>> {
    Ok(vec![
        spec(
            ToolSchema::empty_object(
                "screenshot",
                "Capture the current mobile screen as an image reference.",
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            ToolSchema::empty_object(
                "ui_state",
                "Read current visible UI elements and accessibility labels.",
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "click",
                "Tap a visible UI element by observation index.",
                &["index"],
                json!({"index": {"type": "integer", "description": "UI element index from the current observation"}}),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "click_at",
                "Tap absolute screen coordinates.",
                &["x", "y"],
                json!({
                    "x": {"type": "integer"},
                    "y": {"type": "integer"}
                }),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "click_area",
                "Tap the center of a rectangular screen area.",
                &["x1", "y1", "x2", "y2"],
                json!({
                    "x1": {"type": "integer"},
                    "y1": {"type": "integer"},
                    "x2": {"type": "integer"},
                    "y2": {"type": "integer"}
                }),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "long_press",
                "Long press a visible UI element by observation index.",
                &["index"],
                json!({"index": {"type": "integer"}}),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "long_press_at",
                "Long press absolute screen coordinates.",
                &["x", "y"],
                json!({
                    "x": {"type": "integer"},
                    "y": {"type": "integer"},
                    "duration_ms": {"type": "integer"}
                }),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "type",
                "Type non-secret text into a UI element.",
                &["text"],
                json!({
                    "text": {"type": "string"},
                    "index": {"type": "integer"}
                }),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "type_secret",
                "Type a named secret into the focused field without exposing its value.",
                &["secret_id"],
                json!({
                    "secret_id": {"type": "string"},
                    "index": {"type": "integer"}
                }),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "system_button",
                "Press a device or navigation button.",
                &["button"],
                json!({"button": {"type": "string", "enum": ["back", "home", "enter", "recent"]}}),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "swipe",
                "Swipe between two screen coordinates.",
                &["coordinate", "coordinate2"],
                json!({
                    "coordinate": {"type": "string", "description": "Start coordinate, for example 100,900"},
                    "coordinate2": {"type": "string", "description": "End coordinate, for example 100,200"}
                }),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "wait",
                "Wait for the screen to settle.",
                &["duration"],
                json!({"duration": {"type": "number"}}),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "remember",
                "Store a small task-local fact for the final answer.",
                &["information"],
                json!({"information": {"type": "string"}}),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "complete",
                "Finish the mobile task.",
                &["success", "reason"],
                json!({
                    "success": {"type": "boolean"},
                    "reason": {"type": "string"}
                }),
            )?,
            ToolVisibility::Direct,
        ),
        spec(
            schema(
                "open_app",
                "Open an installed app by display name or package name.",
                &["text"],
                json!({"text": {"type": "string"}}),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "search_database",
                "Search an external business database exposed as a custom tool.",
                &["query"],
                json!({"query": {"type": "string"}}),
            )?,
            ToolVisibility::Searchable,
        ),
        spec(
            schema(
                "raw_adb_shell",
                "Hidden raw shell escape hatch used only to prove visibility enforcement.",
                &["command"],
                json!({"command": {"type": "string"}}),
            )?,
            ToolVisibility::Hidden,
        ),
    ])
}

fn spec(schema: ToolSchema, default_visibility: ToolVisibility) -> ToolSpec {
    ToolSpec {
        schema,
        default_visibility,
    }
}

fn schema(
    name: &str,
    description: &str,
    required: &[&str],
    properties: Value,
) -> AgentCoreResult<ToolSchema> {
    ToolSchema::new(
        name,
        description,
        json!({
            "type": "object",
            "required": required,
            "properties": properties,
            "additionalProperties": false
        }),
    )
}

fn redacted_arguments(action: &str, arguments: Value) -> Value {
    if action != "type_secret" {
        return arguments;
    }

    json!({
        "secret_id": arguments
            .get("secret_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "value": "<redacted>"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_pack_matches_expected_surface_size() {
        assert_eq!(tool_specs().unwrap().len(), expected_action_count());
    }
}
