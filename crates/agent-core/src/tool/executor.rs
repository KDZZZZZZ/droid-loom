use crate::agent_definition::AgentDefinition;
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::tool::{ToolInvocation, ToolOutput};
use crate::tool_permissions::{
    AllowAllToolPermissionPolicy, ToolPermissionContext, ToolPermissionDecision,
    ToolPermissionPolicy,
};
use crate::tool_registry::ToolRegistry;
use crate::tool_result::ToolResult;
use crate::RunMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
    permission_policy: Arc<dyn ToolPermissionPolicy>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            registry,
            permission_policy: Arc::new(AllowAllToolPermissionPolicy),
        }
    }

    pub fn with_permission_policy(
        registry: Arc<ToolRegistry>,
        permission_policy: Arc<dyn ToolPermissionPolicy>,
    ) -> Self {
        Self {
            registry,
            permission_policy,
        }
    }

    pub fn execute_one(
        &self,
        definition: &AgentDefinition,
        call: ToolCall,
    ) -> AgentCoreResult<ToolResult> {
        self.execute_one_result(definition, call)
    }

    pub fn execute_one_message(
        &self,
        definition: &AgentDefinition,
        call: ToolCall,
    ) -> AgentCoreResult<RunMessage> {
        self.execute_one_result(definition, call)?
            .into_run_message()
    }

    pub fn execute_batch(
        &self,
        definition: &AgentDefinition,
        calls: Vec<ToolCall>,
    ) -> AgentCoreResult<Vec<ToolResult>> {
        calls
            .into_iter()
            .map(|call| self.execute_one(definition, call))
            .collect()
    }

    pub fn execute_batch_messages(
        &self,
        definition: &AgentDefinition,
        calls: Vec<ToolCall>,
    ) -> AgentCoreResult<Vec<RunMessage>> {
        self.execute_batch(definition, calls)?
            .into_iter()
            .map(|result| result.into_run_message())
            .collect()
    }

    pub fn execute_batch_parallel(
        &self,
        definition: &AgentDefinition,
        calls: Vec<ToolCall>,
    ) -> AgentCoreResult<Vec<ToolResult>> {
        if calls.len() <= 1 {
            return self.execute_batch(definition, calls);
        }

        std::thread::scope(|scope| {
            let handles = calls
                .into_iter()
                .enumerate()
                .map(|(index, call)| {
                    let executor = self.clone();
                    scope.spawn(move || (index, executor.execute_one(definition, call)))
                })
                .collect::<Vec<_>>();

            let mut indexed_results = Vec::with_capacity(handles.len());
            for handle in handles {
                let (index, result) = handle.join().map_err(|_| {
                    AgentCoreError::Fatal("parallel tool worker panicked".to_string())
                })?;
                indexed_results.push((index, result?));
            }
            indexed_results.sort_by_key(|(index, _)| *index);

            Ok(indexed_results
                .into_iter()
                .map(|(_, result)| result)
                .collect())
        })
    }

    pub fn execute_batch_parallel_messages(
        &self,
        definition: &AgentDefinition,
        calls: Vec<ToolCall>,
    ) -> AgentCoreResult<Vec<RunMessage>> {
        self.execute_batch_parallel(definition, calls)?
            .into_iter()
            .map(|result| result.into_run_message())
            .collect()
    }

    fn execute_one_result(
        &self,
        definition: &AgentDefinition,
        call: ToolCall,
    ) -> AgentCoreResult<ToolResult> {
        let tool = self.registry.get_for_agent(definition, &call.tool_name)?;
        tool.metadata().schema.validate_arguments(&call.arguments)?;

        let permission_context = ToolPermissionContext::new(
            definition.name(),
            call.call_id.clone(),
            call.tool_name.clone(),
            call.arguments.clone(),
        );
        let decision = self.permission_policy.decide(&permission_context)?;

        let Some(arguments) = decision.clone().allowed_arguments(call.arguments.clone()) else {
            return Ok(match decision {
                ToolPermissionDecision::Ask { reason }
                | ToolPermissionDecision::Deny { reason } => {
                    ToolResult::denied(call.call_id, call.tool_name, reason)
                }
                ToolPermissionDecision::Allow { .. } | ToolPermissionDecision::Passthrough => {
                    return Err(AgentCoreError::Fatal(
                        "permission decision did not produce arguments".to_string(),
                    ))
                }
            });
        };

        let invocation = ToolInvocation {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            arguments,
            metadata: call.metadata,
        };

        match tool.invoke(invocation) {
            Ok(ToolOutput { output, metadata }) => {
                let mut result = ToolResult::success(call.call_id, call.tool_name, output);
                result.metadata = metadata;
                Ok(result)
            }
            Err(error) => Ok(ToolResult::failed(
                call.call_id,
                call.tool_name,
                error.to_string(),
                matches!(error, AgentCoreError::Recoverable(_)),
            )),
        }
    }
}

pub trait ToolService: Send + Sync {
    fn call(&self, request: ToolExecutionRequest<'_>) -> AgentCoreResult<RunMessage>;
}

pub trait ToolLayer<S> {
    type Service;

    fn layer(&self, inner: S) -> Self::Service;
}

impl ToolService for ToolExecutor {
    fn call(&self, request: ToolExecutionRequest<'_>) -> AgentCoreResult<RunMessage> {
        let ToolExecutionRequest { definition, call } = request;
        self.execute_one_message(definition, call)
    }
}

pub struct ToolExecutionRequest<'a> {
    pub definition: &'a AgentDefinition,
    pub call: ToolCall,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub metadata: std::collections::BTreeMap<String, Value>,
}

impl ToolCall {
    pub fn new(call_id: impl Into<String>, tool_name: impl Into<String>, arguments: Value) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
            metadata: std::collections::BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_definition::{AgentDefinitionBuilder, ToolVisibility};
    use crate::tool::{Tool, ToolInvocation, ToolMetadata};
    use crate::tool_schema::ToolSchema;
    use serde_json::json;

    #[derive(Debug)]
    struct NamedTool {
        metadata: ToolMetadata,
    }

    impl NamedTool {
        fn new(name: &str) -> Self {
            Self {
                metadata: ToolMetadata::new(
                    ToolSchema::empty_object(name, format!("{name} tool")).unwrap(),
                    ToolVisibility::Direct,
                ),
            }
        }
    }

    impl Tool for NamedTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.metadata
        }

        fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
            Ok(ToolOutput::new(json!({
                "tool": invocation.tool_name,
                "call_id": invocation.call_id
            })))
        }
    }

    #[test]
    fn parallel_batch_preserves_call_order() {
        let definition = AgentDefinitionBuilder::new()
            .name("parallel_agent")
            .system_prompt("prompt")
            .tool_visibility("a", ToolVisibility::Direct)
            .tool_visibility("b", ToolVisibility::Direct)
            .build()
            .unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(NamedTool::new("a")).unwrap();
        registry.register(NamedTool::new("b")).unwrap();
        let executor = ToolExecutor::new(Arc::new(registry));

        let results = executor
            .execute_batch_parallel(
                &definition,
                vec![
                    ToolCall::new("call-a", "a", json!({})),
                    ToolCall::new("call-b", "b", json!({})),
                ],
            )
            .unwrap();

        assert_eq!(results[0].call_id, "call-a");
        assert_eq!(results[1].call_id, "call-b");
        assert_eq!(results[0].tool_name, "a");
        assert_eq!(results[1].tool_name, "b");
    }
}
