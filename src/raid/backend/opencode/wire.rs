use serde::Deserialize;
use serde_json::Value;

use super::types::{CostTier, InterleavedField, InterleavedFieldState, ModelCost, ModelModality, ModelStatus, SdkPackage};

#[derive(Debug, Deserialize)]
pub struct AvailabilityResponse {
    #[serde(default)]
    pub object: Option<String>,
    pub data: Vec<AvailabilityEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum AvailabilityEntry {
    Id(String),
    Object { id: String },
}

pub fn parse_availability_ids(payload: &Value) -> Result<Vec<String>, serde_json::Error> {
    let response: AvailabilityResponse = serde_json::from_value(payload.clone())?;
    Ok(response
        .data
        .into_iter()
        .map(|entry| match entry {
            AvailabilityEntry::Id(id) => id,
            AvailabilityEntry::Object { id } => id,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
pub struct MetadataProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub npm: Option<String>,
    pub models: std::collections::HashMap<String, Value>,
}

#[derive(Debug, Deserialize)]
pub struct WireModel {
    pub id: String,
    pub name: String,
    pub release_date: String,
    pub attachment: bool,
    pub reasoning: bool,
    pub temperature: bool,
    pub tool_call: bool,
    #[serde(default)]
    pub reasoning_options: Option<Vec<Value>>,
    #[serde(default)]
    pub interleaved: Option<WireInterleaved>,
    #[serde(default)]
    pub cost: Option<WireCost>,
    pub limit: WireLimit,
    #[serde(default)]
    pub modalities: Option<WireModalities>,
    #[serde(default)]
    pub status: Option<WireStatus>,
    #[serde(default)]
    pub provider: Option<WireModelProvider>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum WireInterleaved {
    Bool(bool),
    Field(String),
    Object { field: String },
}

#[derive(Debug, Deserialize)]
pub struct WireLimit {
    pub context: u64,
    #[serde(default)]
    pub input: Option<u64>,
    pub output: u64,
}

#[derive(Debug, Deserialize)]
pub struct WireModalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireStatus {
    Alpha,
    Beta,
    Deprecated,
}

#[derive(Debug, Deserialize)]
pub struct WireModelProvider {
    #[serde(default)]
    pub npm: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WireCost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub context_over_200k: Option<WireCostTier>,
}

#[derive(Debug, Deserialize)]
pub struct WireCostTier {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
}

pub fn parse_metadata_provider(payload: &Value) -> Result<MetadataProvider, serde_json::Error> {
    serde_json::from_value(payload.clone())
}

pub fn parse_wire_model(payload: &Value) -> Result<WireModel, serde_json::Error> {
    serde_json::from_value(payload.clone())
}

pub fn normalize_status(status: Option<WireStatus>) -> ModelStatus {
    match status {
        Some(WireStatus::Alpha) => ModelStatus::Alpha,
        Some(WireStatus::Beta) => ModelStatus::Beta,
        Some(WireStatus::Deprecated) => ModelStatus::Deprecated,
        None => ModelStatus::Active,
    }
}

pub fn normalize_interleaved(value: Option<WireInterleaved>) -> InterleavedFieldState {
    match value {
        None | Some(WireInterleaved::Bool(false)) => InterleavedFieldState::Unsupported { supported: false },
        Some(WireInterleaved::Bool(true)) => InterleavedFieldState::Supported {
            supported: true,
            field: None,
        },
        Some(WireInterleaved::Field(field)) | Some(WireInterleaved::Object { field }) => {
            match field.as_str() {
                "reasoning" => InterleavedFieldState::Supported {
                    supported: true,
                    field: Some(InterleavedField::Reasoning),
                },
                "reasoning_content" => InterleavedFieldState::Supported {
                    supported: true,
                    field: Some(InterleavedField::ReasoningContent),
                },
                "reasoning_details" => InterleavedFieldState::Supported {
                    supported: true,
                    field: Some(InterleavedField::ReasoningDetails),
                },
                "reasoning_text" => InterleavedFieldState::Supported {
                    supported: true,
                    field: Some(InterleavedField::ReasoningText),
                },
                _ => InterleavedFieldState::Unsupported { supported: false },
            }
        }
    }
}

pub fn normalize_cost(cost: Option<WireCost>) -> Option<ModelCost> {
    cost.map(|cost| ModelCost {
        input: cost.input,
        output: cost.output,
        cache_read: cost.cache_read,
        cache_write: cost.cache_write,
        context_over_200k: cost.context_over_200k.map(|tier| CostTier {
            input: tier.input,
            output: tier.output,
            cache_read: tier.cache_read,
            cache_write: tier.cache_write,
        }),
    })
}

pub fn parse_modality(value: &str) -> Option<ModelModality> {
    match value {
        "text" => Some(ModelModality::Text),
        "audio" => Some(ModelModality::Audio),
        "image" => Some(ModelModality::Image),
        "video" => Some(ModelModality::Video),
        "pdf" => Some(ModelModality::Pdf),
        _ => None,
    }
}

pub fn normalize_modalities(
    value: Option<WireModalities>,
) -> super::types::Modalities {
    let empty = || super::types::Modalities {
        input: Vec::new(),
        output: Vec::new(),
    };
    let Some(value) = value else {
        return empty();
    };
    super::types::Modalities {
        input: value
            .input
            .iter()
            .filter_map(|entry| parse_modality(entry))
            .collect(),
        output: value
            .output
            .iter()
            .filter_map(|entry| parse_modality(entry))
            .collect(),
    }
}

pub fn parse_sdk_package(value: &str) -> Option<SdkPackage> {
    match value {
        "@ai-sdk/openai" => Some(SdkPackage::OpenAi),
        "@ai-sdk/openai-compatible" => Some(SdkPackage::OpenAiCompatible),
        "@ai-sdk/anthropic" => Some(SdkPackage::Anthropic),
        "@ai-sdk/google" => Some(SdkPackage::Google),
        _ => None,
    }
}
