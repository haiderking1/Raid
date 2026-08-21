use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpenCodePlan {
    Zen,
    Go,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OpenCodeProtocol {
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
    #[serde(rename = "google-generative-ai")]
    GoogleGenerativeAi,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_compatible: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anthropic: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google: Option<serde_json::Map<String, serde_json::Value>>,
}

#[cfg(test)]
mod catalog_types {
    use super::{OpenCodePlan, OpenCodeProtocol, ProviderOptions};
    use serde::{Deserialize, Serialize};

    use crate::backend::opencode::endpoints::protocol_for_sdk_package;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum SdkPackage {
        #[serde(rename = "@ai-sdk/openai")]
        OpenAi,
        #[serde(rename = "@ai-sdk/openai-compatible")]
        OpenAiCompatible,
        #[serde(rename = "@ai-sdk/anthropic")]
        Anthropic,
        #[serde(rename = "@ai-sdk/google")]
        Google,
    }

    impl SdkPackage {
        pub fn protocol(self) -> OpenCodeProtocol {
            protocol_for_sdk_package(self)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ModelModality {
        Text,
        Audio,
        Image,
        Video,
        Pdf,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ModelStatus {
        Active,
        Alpha,
        Beta,
        Deprecated,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum CatalogSource {
        Network,
        Cache,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "kebab-case")]
    pub enum DiagnosticCode {
        MissingMetadata,
        InvalidMetadata,
        UnsupportedProtocol,
        StaleMetadata,
        AmbiguousReasoningMetadata,
        UnsupportedReasoningControl,
        InvalidReasoningOption,
        UnknownReasoningOption,
        InvalidBudgetBounds,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CatalogDiagnostic {
        pub code: DiagnosticCode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub model_id: Option<String>,
        pub detail: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ModelCost {
        pub input: f64,
        pub output: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cache_read: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cache_write: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub context_over_200k: Option<CostTier>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct CostTier {
        pub input: f64,
        pub output: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cache_read: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cache_write: Option<f64>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum InterleavedField {
        Reasoning,
        ReasoningContent,
        ReasoningDetails,
        ReasoningText,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum InterleavedFieldState {
        Unsupported {
            supported: bool,
        },
        Supported {
            supported: bool,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            field: Option<InterleavedField>,
        },
    }

    impl Default for InterleavedFieldState {
        fn default() -> Self {
            Self::Unsupported { supported: false }
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct Modalities {
        pub input: Vec<ModelModality>,
        pub output: Vec<ModelModality>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum ReasoningVariantKind {
        Default,
        Effort,
        Toggle,
        Budget,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ReasoningVariant {
        pub id: String,
        pub label: String,
        pub kind: ReasoningVariantKind,
        #[serde(default)]
        pub provider_options: ProviderOptions,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ResolvedModel {
        pub plan: OpenCodePlan,
        pub plan_label: String,
        pub metadata_provider_id: String,
        pub id: String,
        pub name: String,
        pub sdk_package: SdkPackage,
        pub protocol: OpenCodeProtocol,
        pub context_limit: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub explicit_input_limit: Option<u64>,
        pub output_limit: u64,
        pub tool_call: bool,
        pub reasoning: bool,
        pub modalities: Modalities,
        pub interleaved: InterleavedFieldState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cost: Option<ModelCost>,
        pub status: ModelStatus,
        pub reasoning_variants: Vec<ReasoningVariant>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct OpenCodeCatalog {
        pub plan: OpenCodePlan,
        pub plan_label: String,
        pub metadata_provider_id: String,
        pub models: Vec<ResolvedModel>,
        pub diagnostics: Vec<CatalogDiagnostic>,
        pub metadata_source: CatalogSource,
    }
}

#[cfg(test)]
pub use catalog_types::*;
