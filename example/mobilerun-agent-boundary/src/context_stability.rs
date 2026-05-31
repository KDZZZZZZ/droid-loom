use agent_core::run_message::RunMessage;
use serde_json::json;

const STABILITY_KEY: &str = "context.stability";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextStability {
    StablePrefix,
    TaskPackage,
    DependencyResult,
    VolatileObservation,
}

impl ContextStability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StablePrefix => "stable_prefix",
            Self::TaskPackage => "task_package",
            Self::DependencyResult => "dependency_result",
            Self::VolatileObservation => "volatile_observation",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::StablePrefix => 0,
            Self::TaskPackage => 1,
            Self::DependencyResult => 2,
            Self::VolatileObservation => 3,
        }
    }

    fn from_message(message: &RunMessage) -> Self {
        match message
            .metadata
            .get(STABILITY_KEY)
            .and_then(|value| value.as_str())
        {
            Some("stable_prefix") => Self::StablePrefix,
            Some("task_package") => Self::TaskPackage,
            Some("dependency_result") => Self::DependencyResult,
            _ => Self::VolatileObservation,
        }
    }
}

pub fn mark_stability(mut message: RunMessage, stability: ContextStability) -> RunMessage {
    message
        .metadata
        .insert(STABILITY_KEY.to_string(), json!(stability.as_str()));
    message
}

pub fn order_by_stability(messages: Vec<RunMessage>) -> Vec<RunMessage> {
    let mut indexed = messages.into_iter().enumerate().collect::<Vec<_>>();
    indexed
        .sort_by_key(|(index, message)| (ContextStability::from_message(message).rank(), *index));
    indexed.into_iter().map(|(_, message)| message).collect()
}

pub fn stability_order_labels(messages: &[RunMessage]) -> Vec<&'static str> {
    messages
        .iter()
        .map(|message| ContextStability::from_message(message).as_str())
        .collect()
}

pub fn stable_prefix_id(system_prompt: &str, direct_tool_names: &[String]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in system_prompt.bytes().chain([0]).chain(
        direct_tool_names
            .iter()
            .flat_map(|name| name.bytes().chain([0])),
    ) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("stable-prefix-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::content_block::ContentBlock;
    use agent_core::run_message::RunMessage;

    fn message(text: &str, stability: ContextStability) -> RunMessage {
        mark_stability(
            RunMessage::user(vec![ContentBlock::text(text)]).unwrap(),
            stability,
        )
    }

    #[test]
    fn sorts_context_from_stable_to_volatile() {
        let ordered = order_by_stability(vec![
            message("screen", ContextStability::VolatileObservation),
            message("prefix", ContextStability::StablePrefix),
            message("task", ContextStability::TaskPackage),
        ]);

        assert_eq!(
            stability_order_labels(&ordered),
            vec!["stable_prefix", "task_package", "volatile_observation"]
        );
    }
}
