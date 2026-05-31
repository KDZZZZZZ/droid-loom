use crate::error::{AgentCoreError, AgentCoreResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, Value>,
}

impl ToolSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> AgentCoreResult<Self> {
        let schema = Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            output_schema: None,
            annotations: BTreeMap::new(),
        };
        schema.validate_metadata()?;
        Ok(schema)
    }

    pub fn empty_object(
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> AgentCoreResult<Self> {
        Self::new(
            name,
            description,
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        )
    }

    pub fn with_output_schema(mut self, output_schema: Value) -> Self {
        self.output_schema = Some(output_schema);
        self
    }

    pub fn with_annotation(mut self, key: impl Into<String>, value: Value) -> Self {
        self.annotations.insert(key.into(), value);
        self
    }

    pub fn validate_metadata(&self) -> AgentCoreResult<()> {
        validate_tool_name(&self.name)?;

        if self.description.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(format!(
                "tool `{}` description must not be empty",
                self.name
            )));
        }

        if !self.input_schema.is_object() {
            return Err(AgentCoreError::InvalidConfig(format!(
                "tool `{}` input schema must be a JSON object",
                self.name
            )));
        }

        Ok(())
    }

    pub fn validate_arguments(&self, arguments: &Value) -> AgentCoreResult<()> {
        if !arguments.is_object() {
            return Err(AgentCoreError::InvalidInput(format!(
                "tool `{}` arguments must be a JSON object",
                self.name
            )));
        }

        if let Some(required) = self
            .input_schema
            .get("required")
            .and_then(|value| value.as_array())
        {
            for field in required.iter().filter_map(|value| value.as_str()) {
                if arguments.get(field).is_none() {
                    return Err(AgentCoreError::InvalidInput(format!(
                        "tool `{}` missing required argument `{}`",
                        self.name, field
                    )));
                }
            }
        }

        Ok(())
    }

    pub fn to_function_tool_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.input_schema,
        })
    }
}

pub fn validate_tool_name(name: &str) -> AgentCoreResult<()> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err(AgentCoreError::InvalidConfig(
            "tool name must not be empty".to_string(),
        ));
    }

    if trimmed != name {
        return Err(AgentCoreError::InvalidConfig(format!(
            "tool name `{name}` must not contain leading or trailing whitespace"
        )));
    }

    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(AgentCoreError::InvalidConfig(format!(
            "tool name `{name}` contains unsupported characters"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_required_arguments() {
        let schema = ToolSchema::new(
            "search",
            "Search docs",
            json!({
                "type": "object",
                "required": ["query"],
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        )
        .unwrap();

        assert!(schema
            .validate_arguments(&json!({"query": "agent"}))
            .is_ok());
        assert!(schema.validate_arguments(&json!({})).is_err());
    }
}
