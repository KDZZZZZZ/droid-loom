use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ModelId(pub String);

impl ModelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModelId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModel {
    pub id: ModelId,
    pub provider: String,
    pub api: LlmApi,
    pub capabilities: LlmModelCapabilities,
    pub context_window: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl LlmModel {
    pub fn new(id: impl Into<ModelId>, provider: impl Into<String>, api: LlmApi) -> Self {
        Self {
            id: id.into(),
            provider: provider.into(),
            api,
            capabilities: LlmModelCapabilities::default(),
            context_window: None,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "name")]
pub enum LlmApi {
    OpenAiResponses,
    DeepSeekChat,
    Custom(String),
}

impl LlmApi {
    pub fn as_key(&self) -> &str {
        match self {
            Self::OpenAiResponses => "openai_responses",
            Self::DeepSeekChat => "deepseek_chat",
            Self::Custom(name) => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModelCapabilities {
    pub text_input: bool,
    pub text_output: bool,
    pub image_input: bool,
    pub file_input: bool,
    pub function_tools: bool,
    pub parallel_tool_calls: bool,
    pub streaming: bool,
    pub reasoning: bool,
}

impl Default for LlmModelCapabilities {
    fn default() -> Self {
        Self {
            text_input: true,
            text_output: true,
            image_input: false,
            file_input: false,
            function_tools: true,
            parallel_tool_calls: true,
            streaming: true,
            reasoning: false,
        }
    }
}
