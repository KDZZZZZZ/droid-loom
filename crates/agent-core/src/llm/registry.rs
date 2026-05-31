use crate::error::{AgentCoreError, AgentCoreResult};
use crate::llm_model::{LlmModel, ModelId};
use crate::llm_provider::LlmProvider;
use crate::llm_request::LlmRequest;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct LlmRegistry {
    providers: BTreeMap<String, Arc<dyn LlmProvider>>,
    models: BTreeMap<ModelId, LlmModel>,
}

impl LlmRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_provider<P>(&mut self, provider: P) -> AgentCoreResult<()>
    where
        P: LlmProvider + 'static,
    {
        self.register_provider_arc(Arc::new(provider))
    }

    pub fn register_provider_arc(&mut self, provider: Arc<dyn LlmProvider>) -> AgentCoreResult<()> {
        let api = provider.api().to_string();
        if api.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "llm provider api must not be empty".to_string(),
            ));
        }

        if self.providers.contains_key(&api) {
            return Err(AgentCoreError::InvalidConfig(format!(
                "llm provider api `{api}` already registered"
            )));
        }

        self.providers.insert(api, provider);
        Ok(())
    }

    pub fn register_model(&mut self, model: LlmModel) -> AgentCoreResult<()> {
        if model.id.as_str().trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "model id must not be empty".to_string(),
            ));
        }

        if self.models.contains_key(&model.id) {
            return Err(AgentCoreError::InvalidConfig(format!(
                "model `{}` already registered",
                model.id.as_str()
            )));
        }

        self.models.insert(model.id.clone(), model);
        Ok(())
    }

    pub fn model(&self, model: &ModelId) -> Option<&LlmModel> {
        self.models.get(model)
    }

    pub fn provider_by_api(&self, api: &str) -> AgentCoreResult<Arc<dyn LlmProvider>> {
        self.providers
            .get(api)
            .cloned()
            .ok_or_else(|| AgentCoreError::NotFound(format!("llm provider api `{api}` not found")))
    }

    pub fn provider_for_model(&self, model: &ModelId) -> AgentCoreResult<Arc<dyn LlmProvider>> {
        let model = self.model(model).ok_or_else(|| {
            AgentCoreError::NotFound(format!("model `{}` not found", model.as_str()))
        })?;
        self.provider_by_api(model.api.as_key())
    }

    pub fn provider_for_request(
        &self,
        request: &LlmRequest,
    ) -> AgentCoreResult<Arc<dyn LlmProvider>> {
        if let Some(api) = &request.api {
            return self.provider_by_api(api);
        }

        self.provider_for_model(&request.model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_model::{LlmApi, LlmModel};
    use crate::llm_provider::PreparedLlmRequest;
    use serde_json::json;

    #[derive(Debug)]
    struct MockProvider;

    impl LlmProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }

        fn api(&self) -> &str {
            "mock_api"
        }

        fn prepare_request(&self, _request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest> {
            Ok(PreparedLlmRequest {
                provider: "mock".to_string(),
                api: "mock_api".to_string(),
                endpoint: "mock://local".to_string(),
                headers: BTreeMap::new(),
                body: json!({}),
                metadata: BTreeMap::new(),
            })
        }
    }

    #[test]
    fn resolves_provider_from_model_api() {
        let mut registry = LlmRegistry::new();
        registry.register_provider(MockProvider).unwrap();
        registry
            .register_model(LlmModel::new(
                "mock-model",
                "mock",
                LlmApi::Custom("mock_api".to_string()),
            ))
            .unwrap();

        let request = LlmRequest::new("mock-model");
        let provider = registry.provider_for_request(&request).unwrap();
        assert_eq!(provider.provider_id(), "mock");
    }
}
