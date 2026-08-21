use std::collections::HashSet;

use serde_json::Value;

use super::error::CatalogError;
use super::json::{assert_json_value, snapshot_safe_json};
use super::types::{OpenCodeCatalog, OpenCodePlan, ResolvedModel};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CatalogValidationError(pub String);

pub fn serialize_catalog(catalog: &OpenCodeCatalog) -> Result<Value, CatalogError> {
    validate_catalog_invariants(catalog)?;
    let value = serde_json::to_value(catalog).map_err(|error| {
        CatalogError::with_cause("invalid-catalog", "Catalog failed serialization.", error)
    })?;
    assert_json_value(&value)?;
    Ok(value)
}

pub fn parse_cached_catalog(payload: &Value) -> Result<OpenCodeCatalog, CatalogError> {
    let safe = snapshot_safe_json(payload).map_err(|error| {
        CatalogError::with_cause(
            "invalid-cached-metadata",
            "Cached catalog payload was not valid JSON.",
            error,
        )
    })?;
    let catalog: OpenCodeCatalog = serde_json::from_value(safe.clone()).map_err(|error| {
        CatalogError::with_cause(
            "invalid-cached-metadata",
            "Cached catalog payload failed validation.",
            CatalogError::new("invalid-cached-metadata", error.to_string()),
        )
    })?;
    validate_catalog_invariants(&catalog)?;
    assert_json_value(&safe)?;
    Ok(catalog)
}

pub fn validate_catalog_invariants(catalog: &OpenCodeCatalog) -> Result<(), CatalogError> {
    let expected_label = match catalog.plan {
        OpenCodePlan::Zen => "Zen",
        OpenCodePlan::Go => "Go",
    };
    if catalog.plan_label != expected_label {
        return Err(CatalogError::new(
            "invalid-catalog",
            format!(
                "Catalog plan label '{}' does not match plan '{}'.",
                catalog.plan_label, expected_label
            ),
        ));
    }

    let expected_provider = match catalog.plan {
        OpenCodePlan::Zen => "opencode",
        OpenCodePlan::Go => "opencode-go",
    };
    if catalog.metadata_provider_id != expected_provider {
        return Err(CatalogError::new(
            "invalid-catalog",
            format!(
                "Catalog metadata provider '{}' does not match plan '{}'.",
                catalog.metadata_provider_id, expected_provider
            ),
        ));
    }

    let mut model_ids = HashSet::new();
    for model in &catalog.models {
        validate_model(model, catalog.plan)?;
        if !model_ids.insert(model.id.clone()) {
            return Err(CatalogError::new(
                "invalid-catalog",
                format!("Duplicate model id '{}'.", model.id),
            ));
        }
    }
    Ok(())
}

fn validate_model(model: &ResolvedModel, plan: OpenCodePlan) -> Result<(), CatalogError> {
    if model.plan != plan {
        return Err(CatalogError::new(
            "invalid-catalog",
            format!("Model '{}' plan does not match catalog plan.", model.id),
        ));
    }
    if model.protocol != model.sdk_package.protocol() {
        return Err(CatalogError::new(
            "invalid-catalog",
            format!(
                "Model '{}' SDK package does not match protocol.",
                model.id
            ),
        ));
    }
    let mut variant_ids = HashSet::new();
    for variant in &model.reasoning_variants {
        if !variant_ids.insert(variant.id.clone()) {
            return Err(CatalogError::new(
                "invalid-catalog",
                format!(
                    "Duplicate reasoning variant id '{}' on model '{}'.",
                    variant.id, model.id
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::{
        CatalogSource, InterleavedFieldState, Modalities, ModelModality, ModelStatus, OpenCodeProtocol,
        SdkPackage,
    };

    fn sample_model(id: &str) -> ResolvedModel {
        ResolvedModel {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            id: id.into(),
            name: id.into(),
            sdk_package: SdkPackage::OpenAiCompatible,
            protocol: OpenCodeProtocol::OpenAiCompatible,
            context_limit: 200_000,
            explicit_input_limit: None,
            output_limit: 32_000,
            tool_call: true,
            reasoning: true,
            modalities: Modalities {
                input: vec![ModelModality::Text],
                output: vec![ModelModality::Text],
            },
            interleaved: InterleavedFieldState::Unsupported { supported: false },
            cost: None,
            status: ModelStatus::Active,
            reasoning_variants: Vec::new(),
        }
    }

    fn sample_catalog() -> OpenCodeCatalog {
        OpenCodeCatalog {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            models: vec![sample_model("gpt-test")],
            diagnostics: Vec::new(),
            metadata_source: CatalogSource::Network,
        }
    }

    #[test]
    fn round_trips_valid_catalog() {
        let catalog = sample_catalog();
        let serialized = serialize_catalog(&catalog).expect("serialize");
        let parsed = parse_cached_catalog(&serialized).expect("parse");
        assert_eq!(parsed, catalog);
    }

    #[test]
    fn rejects_duplicate_model_ids() {
        let mut catalog = sample_catalog();
        catalog.models.push(sample_model("gpt-test"));
        assert!(validate_catalog_invariants(&catalog).is_err());
    }
}
