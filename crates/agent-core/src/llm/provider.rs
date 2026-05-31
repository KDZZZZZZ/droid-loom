use crate::error::AgentCoreResult;
use crate::llm_request::LlmRequest;
use crate::llm_stream::{stream_from_events, LlmStream, LlmStreamEvent};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Debug;

pub trait ProviderService: Send + Sync {
    fn call(&self, request: LlmRequest) -> AgentCoreResult<LlmStream>;
}

pub trait ProviderLayer<S> {
    type Service;

    fn layer(&self, inner: S) -> Self::Service;
}

pub trait LlmProvider: Debug + Send + Sync {
    fn provider_id(&self) -> &str;

    fn api(&self) -> &str;

    fn prepare_request(&self, request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest>;

    fn stream(&self, request: &LlmRequest) -> AgentCoreResult<LlmStream> {
        let prepared = self.prepare_request(request)?;
        Ok(stream_from_events(vec![LlmStreamEvent::PreparedRequest {
            provider: prepared.provider.clone(),
            body: prepared.body,
        }]))
    }
}

impl<T> ProviderService for T
where
    T: LlmProvider,
{
    fn call(&self, request: LlmRequest) -> AgentCoreResult<LlmStream> {
        self.stream(&request)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreparedLlmRequest {
    pub provider: String,
    pub api: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    pub body: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}
