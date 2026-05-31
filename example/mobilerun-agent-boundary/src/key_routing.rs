use agent_core::llm_request::LlmRequest;
use agent_core::{AgentCoreError, AgentCoreResult};
use serde_json::json;
use std::collections::BTreeMap;

pub trait SecretResolver {
    fn resolve(&self, env_var: &str) -> Option<String>;
}

#[derive(Clone, Debug, Default)]
pub struct EnvSecretResolver;

impl SecretResolver for EnvSecretResolver {
    fn resolve(&self, env_var: &str) -> Option<String> {
        std::env::var(env_var)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default)]
pub struct StaticSecretResolver {
    secrets: BTreeMap<String, String>,
}

impl StaticSecretResolver {
    pub fn new(secrets: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self {
            secrets: secrets
                .into_iter()
                .map(|(name, value)| (name.into(), value.into()))
                .collect(),
        }
    }
}

impl SecretResolver for StaticSecretResolver {
    fn resolve(&self, env_var: &str) -> Option<String> {
        self.secrets.get(env_var).cloned()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiKeyRoute {
    pub account_label: String,
    pub env_var: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRoutePlan {
    pub stable_prefix_id: String,
    pub stable_route: ApiKeyRoute,
    pub general_route: ApiKeyRoute,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutedRequest {
    pub request: LlmRequest,
    pub account_label: String,
    pub env_var: String,
}

impl KeyRoutePlan {
    pub fn new(stable_prefix_id: impl Into<String>) -> Self {
        Self {
            stable_prefix_id: stable_prefix_id.into(),
            stable_route: ApiKeyRoute {
                account_label: "stable-prefix-account".to_string(),
                env_var: "MOBILERUN_STABLE_PREFIX_API_KEY".to_string(),
            },
            general_route: ApiKeyRoute {
                account_label: "general-pool-account".to_string(),
                env_var: "MOBILERUN_GENERAL_POOL_API_KEY".to_string(),
            },
        }
    }

    pub fn route_for_prefix(&self, prefix_id: &str) -> &ApiKeyRoute {
        if prefix_id == self.stable_prefix_id {
            &self.stable_route
        } else {
            &self.general_route
        }
    }

    pub fn apply<R: SecretResolver>(
        &self,
        mut request: LlmRequest,
        prefix_id: &str,
        resolver: &R,
    ) -> AgentCoreResult<RoutedRequest> {
        let route = self.route_for_prefix(prefix_id);
        let api_key = resolver.resolve(&route.env_var).ok_or_else(|| {
            AgentCoreError::InvalidConfig(format!(
                "missing API key environment variable: {}",
                route.env_var
            ))
        })?;

        request
            .headers
            .insert("Authorization".to_string(), format!("Bearer {api_key}"));
        request.metadata.insert(
            "key_route.account".to_string(),
            json!(route.account_label.clone()),
        );
        request.metadata.insert(
            "key_route.env_var".to_string(),
            json!(route.env_var.clone()),
        );
        request
            .metadata
            .insert("key_route.prefix_id".to_string(), json!(prefix_id));

        Ok(RoutedRequest {
            request,
            account_label: route.account_label.clone(),
            env_var: route.env_var.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::llm_deepseek_chat::DeepSeekChatProvider;
    use agent_core::llm_provider::LlmProvider;
    use agent_core::llm_request::{LlmContentPart, LlmInputItem, LlmMessageRole};

    #[test]
    fn stable_and_general_prefixes_route_to_different_key_slots() {
        let plan = KeyRoutePlan::new("stable-prefix-a");
        let resolver = StaticSecretResolver::new([
            ("MOBILERUN_STABLE_PREFIX_API_KEY", "stable-route-fixture"),
            ("MOBILERUN_GENERAL_POOL_API_KEY", "general-route-fixture"),
        ]);

        let stable = plan
            .apply(LlmRequest::new("model"), "stable-prefix-a", &resolver)
            .unwrap();
        let general = plan
            .apply(LlmRequest::new("model"), "volatile-prefix-b", &resolver)
            .unwrap();

        assert_eq!(stable.account_label, "stable-prefix-account");
        assert_eq!(general.account_label, "general-pool-account");
        assert_ne!(
            stable.request.headers.get("Authorization"),
            general.request.headers.get("Authorization")
        );
    }

    #[test]
    fn provider_prepare_keeps_route_keys_in_headers_not_metadata() {
        let stable_key = "stable-route-fixture";
        let general_key = "general-route-fixture";
        let plan = KeyRoutePlan::new("stable-prefix-a");
        let resolver = StaticSecretResolver::new([
            ("MOBILERUN_STABLE_PREFIX_API_KEY", stable_key),
            ("MOBILERUN_GENERAL_POOL_API_KEY", general_key),
        ]);

        let stable = plan
            .apply(chat_request(), "stable-prefix-a", &resolver)
            .unwrap();
        let general = plan
            .apply(chat_request(), "volatile-prefix-b", &resolver)
            .unwrap();
        let provider = DeepSeekChatProvider::new().with_endpoint("http://127.0.0.1/mock-chat");

        let stable_prepared = provider.prepare_request(&stable.request).unwrap();
        let general_prepared = provider.prepare_request(&general.request).unwrap();

        assert_eq!(
            stable_prepared.headers.get("Authorization"),
            Some(&format!("Bearer {stable_key}"))
        );
        assert_eq!(
            general_prepared.headers.get("Authorization"),
            Some(&format!("Bearer {general_key}"))
        );
        assert_ne!(
            stable_prepared.headers.get("Authorization"),
            general_prepared.headers.get("Authorization")
        );

        assert_eq!(
            stable.request.metadata["key_route.account"],
            "stable-prefix-account"
        );
        assert_eq!(
            stable.request.metadata["key_route.prefix_id"],
            "stable-prefix-a"
        );
        assert_eq!(
            general.request.metadata["key_route.account"],
            "general-pool-account"
        );
        assert_eq!(
            general.request.metadata["key_route.prefix_id"],
            "volatile-prefix-b"
        );

        let stable_metadata = serde_json::to_string(&stable.request.metadata).unwrap();
        let general_metadata = serde_json::to_string(&general.request.metadata).unwrap();
        let stable_body = stable_prepared.body.to_string();
        let general_body = general_prepared.body.to_string();

        for serialized in [
            stable_metadata.as_str(),
            general_metadata.as_str(),
            stable_body.as_str(),
            general_body.as_str(),
        ] {
            assert!(!serialized.contains(stable_key));
            assert!(!serialized.contains(general_key));
        }
        assert!(stable_metadata.contains("MOBILERUN_STABLE_PREFIX_API_KEY"));
        assert!(general_metadata.contains("MOBILERUN_GENERAL_POOL_API_KEY"));
        assert!(stable_prepared.metadata.is_empty());
        assert!(general_prepared.metadata.is_empty());
    }

    fn chat_request() -> LlmRequest {
        let mut request = LlmRequest::new("mimo-v2.5-pro");
        request.instructions = Some("stable Android agent prefix".to_string());
        request.input.push(LlmInputItem::Message {
            role: LlmMessageRole::User,
            content: vec![LlmContentPart::Text {
                text: "observe current app state".to_string(),
            }],
            metadata: BTreeMap::new(),
        });
        request
    }
}
