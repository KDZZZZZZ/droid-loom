use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{AgentCoreError, AgentCoreResult};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDefinition {
    name: String,
    system_prompt: String,
    tool_visibility: BTreeMap<String, ToolVisibility>,
}

impl AgentDefinition {
    pub fn new(
        name: impl Into<String>,
        system_prompt: impl Into<String>,
        tool_visibility: BTreeMap<String, ToolVisibility>,
    ) -> AgentCoreResult<Self> {
        let definition = Self {
            name: name.into(),
            system_prompt: system_prompt.into(),
            tool_visibility,
        };
        definition.validate()?;
        Ok(definition)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    pub fn tool_visibility(&self) -> &BTreeMap<String, ToolVisibility> {
        &self.tool_visibility
    }

    pub fn validate(&self) -> AgentCoreResult<()> {
        if self.name.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "agent name must not be empty".to_string(),
            ));
        }

        if self.system_prompt.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "system prompt must not be empty".to_string(),
            ));
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolVisibility {
    Direct,
    Searchable,
    Hidden,
}

#[derive(Clone, Debug, Default)]
pub struct AgentDefinitionBuilder {
    name: Option<String>,
    system_prompt: Option<String>,
    tool_visibility: BTreeMap<String, ToolVisibility>,
}

impl AgentDefinitionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(system_prompt.into());
        self
    }

    pub fn tool_visibility(
        mut self,
        tool_name: impl Into<String>,
        visibility: ToolVisibility,
    ) -> Self {
        self.tool_visibility.insert(tool_name.into(), visibility);
        self
    }

    pub fn build(self) -> AgentCoreResult<AgentDefinition> {
        AgentDefinition::new(
            self.name.unwrap_or_default(),
            self.system_prompt.unwrap_or_default(),
            self.tool_visibility,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_minimal_definition() {
        let definition = AgentDefinitionBuilder::new()
            .name("coder")
            .system_prompt("You write code.")
            .tool_visibility("search", ToolVisibility::Searchable)
            .build()
            .unwrap();

        assert_eq!(definition.name(), "coder");
        assert_eq!(definition.system_prompt(), "You write code.");
        assert_eq!(
            definition.tool_visibility().get("search"),
            Some(&ToolVisibility::Searchable)
        );
    }

    #[test]
    fn rejects_empty_definition() {
        let err = AgentDefinitionBuilder::new().build().unwrap_err();
        assert!(matches!(err, AgentCoreError::InvalidConfig(_)));
    }
}
