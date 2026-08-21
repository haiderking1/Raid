use crate::backend::opencode::OpenCodePlan;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectProvider {
    pub id: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub plan: OpenCodePlan,
}

pub const PROVIDERS: &[ConnectProvider] = &[
    ConnectProvider {
        id: "opencode",
        label: "OpenCode Zen",
        description: "opencode.ai/zen",
        plan: OpenCodePlan::Zen,
    },
    ConnectProvider {
        id: "opencode-go",
        label: "OpenCode Go",
        description: "opencode.ai/zen/go",
        plan: OpenCodePlan::Go,
    },
];

pub fn provider_by_id(id: &str) -> Option<&'static ConnectProvider> {
    PROVIDERS.iter().find(|provider| provider.id == id)
}

pub fn plan_for_provider_id(id: &str) -> OpenCodePlan {
    provider_by_id(id)
        .map(|provider| provider.plan)
        .unwrap_or(OpenCodePlan::Go)
}

#[cfg(test)]
fn provider_label(id: &str) -> &str {
    provider_by_id(id)
        .map(|provider| provider.label)
        .unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::{provider_label, PROVIDERS};

    #[test]
    fn provider_label_falls_back_to_id() {
        assert_eq!(provider_label("opencode-go"), PROVIDERS[1].label);
        assert_eq!(provider_label("unknown"), "unknown");
    }
}
