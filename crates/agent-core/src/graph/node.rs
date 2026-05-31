use crate::error::AgentCoreResult;
use crate::run_message::RunMessage;
use serde::{Deserialize, Serialize};

pub type NodeId = String;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    id: NodeId,
    label: Option<String>,
    action: GraphNodeAction,
    terminal: bool,
}

impl GraphNode {
    pub fn new(id: impl Into<NodeId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            action: GraphNodeAction::Noop,
            terminal: false,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_action(mut self, action: GraphNodeAction) -> Self {
        self.action = action;
        self
    }

    pub fn terminal(mut self, terminal: bool) -> Self {
        self.terminal = terminal;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn action(&self) -> &GraphNodeAction {
        &self.action
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal
    }

    pub fn execute<F>(
        &self,
        input: NodeExecutionInput,
        mut emit: F,
    ) -> AgentCoreResult<NodeExecutionResult>
    where
        F: FnMut(RunMessage) -> AgentCoreResult<()>,
    {
        let mut emitted_messages = 0;

        match &self.action {
            GraphNodeAction::Noop | GraphNodeAction::Custom(_) => {}
            GraphNodeAction::PassthroughInput => {
                for message in input.input_messages {
                    emit(message)?;
                    emitted_messages += 1;
                }
            }
            GraphNodeAction::EmitMessages(messages) => {
                for message in messages.iter().cloned() {
                    emit(message)?;
                    emitted_messages += 1;
                }
            }
            GraphNodeAction::Agent { agent_name } => {
                return Err(crate::error::AgentCoreError::InvalidConfig(format!(
                    "agent node `{agent_name}` requires an external node executor"
                )));
            }
        }

        Ok(NodeExecutionResult {
            node_id: self.id.clone(),
            status: NodeExecutionStatus::Completed,
            emitted_messages,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum GraphNodeAction {
    Noop,
    PassthroughInput,
    EmitMessages(Vec<RunMessage>),
    Agent { agent_name: String },
    Custom(String),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeExecutionInput {
    pub input_messages: Vec<RunMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExecutionResult {
    pub node_id: NodeId,
    pub status: NodeExecutionStatus,
    pub emitted_messages: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeExecutionStatus {
    Completed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_node_completes_without_messages() {
        let node = GraphNode::new("start");

        let result = node
            .execute(NodeExecutionInput::default(), |_| {
                unreachable!("noop should not emit")
            })
            .unwrap();

        assert_eq!(result.node_id, "start");
        assert_eq!(result.emitted_messages, 0);
        assert_eq!(result.status, NodeExecutionStatus::Completed);
    }

    #[test]
    fn passthrough_node_emits_input_messages() {
        let node = GraphNode::new("input").with_action(GraphNodeAction::PassthroughInput);
        let message =
            RunMessage::user(vec![crate::content_block::ContentBlock::text("hi")]).unwrap();
        let mut emitted = Vec::new();

        let result = node
            .execute(
                NodeExecutionInput {
                    input_messages: vec![message.clone()],
                },
                |message| {
                    emitted.push(message);
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(result.emitted_messages, 1);
        assert_eq!(emitted, vec![message]);
    }
}
