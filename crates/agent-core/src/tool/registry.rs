use crate::agent_definition::{AgentDefinition, ToolVisibility};
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::tool::{Tool, ToolMetadata};
use crate::tool_schema::ToolSchema;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(&mut self, tool: T) -> AgentCoreResult<()>
    where
        T: Tool + 'static,
    {
        self.register_arc(Arc::new(tool))
    }

    pub fn register_arc(&mut self, tool: Arc<dyn Tool>) -> AgentCoreResult<()> {
        let name = tool.metadata().name().to_string();
        tool.metadata().schema.validate_metadata()?;

        if self.tools.contains_key(&name) {
            return Err(AgentCoreError::InvalidConfig(format!(
                "tool `{name}` already registered"
            )));
        }

        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn get_for_agent(
        &self,
        definition: &AgentDefinition,
        name: &str,
    ) -> AgentCoreResult<Arc<dyn Tool>> {
        let tool = self
            .get(name)
            .ok_or_else(|| AgentCoreError::NotFound(format!("tool `{name}` not found")))?;

        if self.visibility_for_agent(definition, tool.metadata()) == ToolVisibility::Hidden {
            return Err(AgentCoreError::PermissionDenied(format!(
                "tool `{}` is hidden for agent `{}`",
                name,
                definition.name()
            )));
        }

        Ok(tool)
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    pub fn visibility_for_agent(
        &self,
        definition: &AgentDefinition,
        metadata: &ToolMetadata,
    ) -> ToolVisibility {
        definition
            .tool_visibility()
            .get(metadata.name())
            .copied()
            .unwrap_or(metadata.default_visibility)
    }

    pub fn direct_tools(&self, definition: &AgentDefinition) -> Vec<Arc<dyn Tool>> {
        self.tools
            .values()
            .filter(|tool| {
                self.visibility_for_agent(definition, tool.metadata()) == ToolVisibility::Direct
            })
            .cloned()
            .collect()
    }

    pub fn direct_schemas(&self, definition: &AgentDefinition) -> Vec<ToolSchema> {
        self.direct_tools(definition)
            .into_iter()
            .map(|tool| tool.metadata().schema.clone())
            .collect()
    }

    pub fn searchable_tools(&self, definition: &AgentDefinition) -> Vec<Arc<dyn Tool>> {
        self.tools
            .values()
            .filter(|tool| {
                self.visibility_for_agent(definition, tool.metadata()) == ToolVisibility::Searchable
            })
            .cloned()
            .collect()
    }

    pub fn search(
        &self,
        definition: &AgentDefinition,
        query: &str,
        limit: usize,
    ) -> Vec<ToolSchema> {
        let normalized_query = query.trim().to_ascii_lowercase();

        self.searchable_tools(definition)
            .into_iter()
            .filter(|tool| {
                if normalized_query.is_empty() {
                    return true;
                }

                let schema = &tool.metadata().schema;
                schema.name.to_ascii_lowercase().contains(&normalized_query)
                    || schema
                        .description
                        .to_ascii_lowercase()
                        .contains(&normalized_query)
            })
            .take(limit)
            .map(|tool| tool.metadata().schema.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_definition::AgentDefinitionBuilder;
    use crate::tool::{ToolInvocation, ToolMetadata, ToolOutput};
    use serde_json::json;

    #[derive(Debug)]
    struct MockTool {
        metadata: ToolMetadata,
    }

    impl MockTool {
        fn new(name: &str, visibility: ToolVisibility) -> Self {
            Self {
                metadata: ToolMetadata::new(
                    ToolSchema::empty_object(name, format!("{name} tool")).unwrap(),
                    visibility,
                ),
            }
        }
    }

    impl Tool for MockTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.metadata
        }

        fn invoke(&self, _invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
            Ok(ToolOutput::new(json!({"ok": true})))
        }
    }

    #[test]
    fn resolves_visibility_from_agent_definition() {
        let mut registry = ToolRegistry::new();
        registry
            .register(MockTool::new("search", ToolVisibility::Searchable))
            .unwrap();
        registry
            .register(MockTool::new("edit", ToolVisibility::Hidden))
            .unwrap();

        let definition = AgentDefinitionBuilder::new()
            .name("coder")
            .system_prompt("prompt")
            .tool_visibility("search", ToolVisibility::Direct)
            .build()
            .unwrap();

        let direct = registry.direct_schemas(&definition);
        assert_eq!(direct.len(), 1);
        assert_eq!(direct[0].name, "search");
        assert!(registry.get_for_agent(&definition, "edit").is_err());
    }
}
