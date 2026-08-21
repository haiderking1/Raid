use super::types::OpenCodePlan;

pub fn plan_endpoints(plan: OpenCodePlan) -> PlanEndpoints {
    match plan {
        OpenCodePlan::Zen => PlanEndpoints {
            id: OpenCodePlan::Zen,
            label: "Zen",
            metadata_provider_id: "opencode",
            base_url: "https://opencode.ai/zen/v1",
            models_url: "https://opencode.ai/zen/v1/models",
        },
        OpenCodePlan::Go => PlanEndpoints {
            id: OpenCodePlan::Go,
            label: "Go",
            metadata_provider_id: "opencode-go",
            base_url: "https://opencode.ai/zen/go/v1",
            models_url: "https://opencode.ai/zen/go/v1/models",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanEndpoints {
    pub id: OpenCodePlan,
    pub label: &'static str,
    pub metadata_provider_id: &'static str,
    pub base_url: &'static str,
    pub models_url: &'static str,
}

pub const METADATA_URL: &str = "https://models.opencode.ai/api.json";
pub const DEFAULT_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[cfg(test)]
use super::types::{OpenCodeProtocol, SdkPackage};

#[cfg(test)]
pub const PLAN_IDS: [OpenCodePlan; 2] = [OpenCodePlan::Zen, OpenCodePlan::Go];

#[cfg(test)]
pub fn metadata_provider_id(plan: OpenCodePlan) -> &'static str {
    plan_endpoints(plan).metadata_provider_id
}

#[cfg(test)]
pub fn protocol_for_sdk_package(sdk: SdkPackage) -> OpenCodeProtocol {
    match sdk {
        SdkPackage::OpenAi => OpenCodeProtocol::OpenAiResponses,
        SdkPackage::OpenAiCompatible => OpenCodeProtocol::OpenAiCompatible,
        SdkPackage::Anthropic => OpenCodeProtocol::AnthropicMessages,
        SdkPackage::Google => OpenCodeProtocol::GoogleGenerativeAi,
    }
}
