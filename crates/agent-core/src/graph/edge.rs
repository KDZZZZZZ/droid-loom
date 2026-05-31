use crate::content_block::ContentBlock;
use crate::error::AgentCoreResult;
use crate::graph_node::NodeId;
use crate::graph_state::GraphStateView;
use crate::run_message::{MessageRole, RunMessage};
use serde::{Deserialize, Serialize};

pub type EdgeId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    id: EdgeId,
    source_node_id: NodeId,
    target_node_id: NodeId,
    inherit_policy: ContextInheritPolicy,
    activation_condition: ActivationCondition,
    priority: i32,
}

impl GraphEdge {
    pub fn new(
        id: impl Into<EdgeId>,
        source_node_id: impl Into<NodeId>,
        target_node_id: impl Into<NodeId>,
    ) -> Self {
        Self {
            id: id.into(),
            source_node_id: source_node_id.into(),
            target_node_id: target_node_id.into(),
            inherit_policy: ContextInheritPolicy::Full,
            activation_condition: ActivationCondition::OnAnyMessage,
            priority: 0,
        }
    }

    pub fn with_inherit_policy(mut self, inherit_policy: ContextInheritPolicy) -> Self {
        self.inherit_policy = inherit_policy;
        self
    }

    pub fn with_activation_condition(mut self, activation_condition: ActivationCondition) -> Self {
        self.activation_condition = activation_condition;
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn source_node_id(&self) -> &str {
        &self.source_node_id
    }

    pub fn target_node_id(&self) -> &str {
        &self.target_node_id
    }

    pub fn inherit_policy(&self) -> &ContextInheritPolicy {
        &self.inherit_policy
    }

    pub fn activation_condition(&self) -> &ActivationCondition {
        &self.activation_condition
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub fn evaluate(
        &self,
        source_message: &RunMessage,
        source_message_version: u64,
        state: &GraphStateView<'_>,
    ) -> AgentCoreResult<EdgeDecision> {
        if self.activation_condition.matches(
            &self.source_node_id,
            source_message,
            source_message_version,
            state,
        ) {
            Ok(EdgeDecision::Activate {
                target_node_id: self.target_node_id.clone(),
                inherit_policy: self.inherit_policy.clone(),
            })
        } else {
            Ok(EdgeDecision::Sleep)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum ContextInheritPolicy {
    Full,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum ActivationCondition {
    OnAnyMessage,
    Always,
    Never,
    MessageRoleIs { role: MessageRole },
    MessageHasText,
    MessageHasToolCall,
    MessageHasToolResult { is_error: Option<bool> },
    SourceVersionAtLeast { version: u64 },
    SourceMessageCountAtLeast { count: usize },
}

impl ActivationCondition {
    pub fn matches(
        &self,
        source_node_id: &str,
        source_message: &RunMessage,
        source_message_version: u64,
        state: &GraphStateView<'_>,
    ) -> bool {
        match self {
            Self::OnAnyMessage | Self::Always => true,
            Self::Never => false,
            Self::MessageRoleIs { role } => source_message.role == *role,
            Self::MessageHasText => source_message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::Text { .. })),
            Self::MessageHasToolCall => source_message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolCall { .. })),
            Self::MessageHasToolResult { is_error } => source_message.content.iter().any(|block| {
                matches!(
                    (block, is_error),
                    (ContentBlock::ToolResult { .. }, None)
                        | (ContentBlock::ToolResult { is_error: true, .. }, Some(true))
                        | (
                            ContentBlock::ToolResult {
                                is_error: false,
                                ..
                            },
                            Some(false)
                        )
                )
            }),
            Self::SourceVersionAtLeast { version } => source_message_version >= *version,
            Self::SourceMessageCountAtLeast { count } => {
                state.message_count(source_node_id) >= *count
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum EdgeDecision {
    Sleep,
    Activate {
        target_node_id: NodeId,
        inherit_policy: ContextInheritPolicy,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;
    use crate::graph_state::{GraphState, GraphStateBudget};
    use uuid::Uuid;

    fn text_message() -> RunMessage {
        RunMessage::assistant(vec![ContentBlock::text("done")]).unwrap()
    }

    #[test]
    fn edge_activates_on_any_source_message() {
        let edge = GraphEdge::new("e1", "source", "target");
        let state = GraphState::new(Uuid::new_v4(), GraphStateBudget::unlimited());

        let decision = edge.evaluate(&text_message(), 1, &state.view()).unwrap();

        assert_eq!(
            decision,
            EdgeDecision::Activate {
                target_node_id: "target".to_string(),
                inherit_policy: ContextInheritPolicy::Full,
            }
        );
    }

    #[test]
    fn edge_sleeps_when_message_predicate_is_false() {
        let edge = GraphEdge::new("e1", "source", "target")
            .with_activation_condition(ActivationCondition::MessageHasToolCall);
        let state = GraphState::new(Uuid::new_v4(), GraphStateBudget::unlimited());

        let decision = edge.evaluate(&text_message(), 1, &state.view()).unwrap();

        assert_eq!(decision, EdgeDecision::Sleep);
    }
}
