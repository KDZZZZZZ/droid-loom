use crate::error::AgentCoreResult;
use crate::llm_provider::{PreparedLlmRequest, ProviderService};
use crate::llm_registry::LlmRegistry;
use crate::llm_request::LlmRequest;
use crate::llm_stream::LlmStream;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct LlmService {
    registry: Arc<LlmRegistry>,
}

impl LlmService {
    pub fn new(registry: Arc<LlmRegistry>) -> Self {
        Self { registry }
    }

    pub fn registry(&self) -> &LlmRegistry {
        &self.registry
    }

    pub fn prepare_request(&self, request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest> {
        let provider = self.registry.provider_for_request(request)?;
        provider.prepare_request(request)
    }

    pub fn stream(&self, request: &LlmRequest) -> AgentCoreResult<LlmStream> {
        let provider = self.registry.provider_for_request(request)?;
        provider.stream(request)
    }
}

impl ProviderService for LlmService {
    fn call(&self, request: LlmRequest) -> AgentCoreResult<LlmStream> {
        self.stream(&request)
    }
}
