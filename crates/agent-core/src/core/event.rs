use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CoreEvent {
    AgentStarted {
        agent_id: Uuid,
        agent_name: String,
        graph_name: String,
    },
    AgentEnded {
        agent_id: Uuid,
        agent_name: String,
        status: String,
    },
    GraphStarted {
        run_id: Uuid,
        graph_name: String,
    },
    GraphEnded {
        run_id: Uuid,
        graph_name: String,
        status: String,
    },
    NodeStarted {
        run_id: Uuid,
        node_id: String,
    },
    NodeEnded {
        run_id: Uuid,
        node_id: String,
        emitted_messages: usize,
    },
    MessageEmitted {
        run_id: Uuid,
        node_id: String,
        message_id: Uuid,
    },
    HookEmitted {
        name: String,
        data: Value,
    },
    Error {
        run_id: Option<Uuid>,
        message: String,
        recoverable: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EventLog {
    events: Vec<CoreEvent>,
}

impl EventLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, event: CoreEvent) {
        self.events.push(event);
    }

    pub fn extend(&mut self, events: impl IntoIterator<Item = CoreEvent>) {
        self.events.extend(events);
    }

    pub fn events(&self) -> &[CoreEvent] {
        &self.events
    }

    pub fn into_events(self) -> Vec<CoreEvent> {
        self.events
    }
}
