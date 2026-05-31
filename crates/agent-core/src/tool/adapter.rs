use crate::error::{AgentCoreError, AgentCoreResult};
use crate::tool::Tool;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub trait ToolAdapter: Send + Sync {
    fn adapter_name(&self) -> &str;

    fn load_tools(&self) -> AgentCoreResult<Vec<Arc<dyn Tool>>>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAdapterMetadata {
    pub name: String,
    pub source: String,
}

#[derive(Default)]
pub struct ToolAdapterRegistry {
    adapters: BTreeMap<String, Arc<dyn ToolAdapter>>,
}

impl ToolAdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<A>(&mut self, adapter: A) -> AgentCoreResult<()>
    where
        A: ToolAdapter + 'static,
    {
        self.register_arc(Arc::new(adapter))
    }

    pub fn register_arc(&mut self, adapter: Arc<dyn ToolAdapter>) -> AgentCoreResult<()> {
        let name = adapter.adapter_name().to_string();
        if name.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "tool adapter name must not be empty".to_string(),
            ));
        }

        if self.adapters.contains_key(&name) {
            return Err(AgentCoreError::InvalidConfig(format!(
                "tool adapter `{name}` already registered"
            )));
        }

        self.adapters.insert(name, adapter);
        Ok(())
    }

    pub fn load_all(&self) -> AgentCoreResult<Vec<Arc<dyn Tool>>> {
        let mut tools = Vec::new();
        for adapter in self.adapters.values() {
            tools.extend(adapter.load_tools()?);
        }
        Ok(tools)
    }
}
