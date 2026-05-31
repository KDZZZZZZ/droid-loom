use crate::agent::Agent;
use crate::agent_definition::AgentDefinition;
use crate::error::AgentCoreResult;
use crate::graph_runner::GraphRunner;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct AgentServices {
    pub graph_runner: GraphRunner,
}

#[derive(Clone, Debug, Default)]
pub struct AgentFactory {
    services: AgentServices,
}

impl AgentFactory {
    pub fn new(services: AgentServices) -> Self {
        Self { services }
    }

    pub fn create(&self, definition: AgentDefinition) -> AgentCoreResult<Agent> {
        Self::validate_definition(&definition)?;
        Ok(Agent::new(Arc::new(definition), self.services.clone()))
    }

    fn validate_definition(definition: &AgentDefinition) -> AgentCoreResult<()> {
        definition.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_definition::AgentDefinitionBuilder;

    #[test]
    fn rejects_empty_name() {
        let definition = AgentDefinitionBuilder::new()
            .system_prompt("prompt")
            .build()
            .unwrap_err();

        assert!(matches!(
            definition,
            crate::error::AgentCoreError::InvalidConfig(_)
        ));
    }

    #[test]
    fn creates_agent_from_valid_definition() {
        let definition = AgentDefinitionBuilder::new()
            .name("coder")
            .system_prompt("prompt")
            .build()
            .unwrap();

        let agent = AgentFactory::default().create(definition).unwrap();
        assert_eq!(agent.definition().name(), "coder");
    }
}
