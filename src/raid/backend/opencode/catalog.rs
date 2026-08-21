use std::collections::HashSet;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use super::cache::MetadataCache;
use super::endpoints::{plan_endpoints, DEFAULT_REQUEST_TIMEOUT, METADATA_URL};
use super::error::CatalogError;
use super::reasoning::{derive_reasoning_variants, parse_reasoning_options};
use super::types::{
    CatalogDiagnostic, CatalogSource, DiagnosticCode, ModelStatus, OpenCodeCatalog, OpenCodePlan,
    ResolvedModel,
};
use super::validate::{parse_cached_catalog, serialize_catalog};
use super::wire::{
    normalize_cost, normalize_interleaved, normalize_modalities, normalize_status, parse_availability_ids,
    parse_metadata_provider, parse_sdk_package, parse_wire_model, MetadataProvider,
};

pub struct LoadCatalogOptions<'a> {
    pub plan: OpenCodePlan,
    pub api_key: Option<&'a str>,
    pub include_deprecated: bool,
    pub timeout: Duration,
    pub cache: Option<&'a dyn MetadataCache>,
    pub http: &'a dyn CatalogHttp,
}

#[async_trait]
pub trait CatalogHttp: Send + Sync {
    async fn get_json(
        &self,
        url: &str,
        authorize: bool,
        api_key: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, CatalogError>;
}

pub struct ReqwestCatalogHttp {
    client: Client,
}

impl Default for ReqwestCatalogHttp {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(DEFAULT_REQUEST_TIMEOUT)
                .build()
                .expect("reqwest client"),
        }
    }
}

#[async_trait]
impl CatalogHttp for ReqwestCatalogHttp {
    async fn get_json(
        &self,
        url: &str,
        authorize: bool,
        api_key: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, CatalogError> {
        let mut request = self
            .client
            .get(url)
            .header("accept", "application/json")
            .timeout(timeout);
        if authorize {
            let Some(api_key) = api_key.filter(|key| !key.is_empty()) else {
                return Err(CatalogError::new(
                    "missing-api-key",
                    "An API key is required to load model availability.",
                ));
            };
            request = request.bearer_auth(api_key);
        }

        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                CatalogError::new("timeout", "Request timed out.")
            } else if error.is_connect() || error.is_request() {
                CatalogError::with_cause("fetch-failed", "Failed to fetch catalog data.", error)
            } else {
                CatalogError::with_cause("fetch-failed", "Network request failed.", error)
            }
        })?;

        if !response.status().is_success() {
            return Err(CatalogError::new(
                "http-error",
                format!("Request to {url} failed with status {}.", response.status()),
            ));
        }

        response.json().await.map_err(|error| {
            CatalogError::with_cause(
                "invalid-json",
                format!("Response from {url} was not valid JSON."),
                error,
            )
        })
    }
}

pub async fn load_catalog(options: LoadCatalogOptions<'_>) -> Result<OpenCodeCatalog, CatalogError> {
    let endpoints = plan_endpoints(options.plan);
    match load_catalog_live(&options, &endpoints).await {
        Ok(catalog) => {
            if let Some(cache) = options.cache {
                if let Ok(serialized) = serialize_catalog(&catalog) {
                    if let Ok(validated) = parse_cached_catalog(&serialized) {
                        let _ = cache.write(options.plan, &validated);
                    }
                }
            }
            Ok(catalog)
        }
        Err(error) if is_recoverable(&error) => {
            let Some(cache) = options.cache else {
                return Err(error);
            };
            match cache.read(options.plan)? {
                Some(catalog) if catalog.plan == options.plan => Ok(with_stale_diagnostic(catalog, &error)),
                Some(_) => Err(CatalogError::stale_cache(
                    "Catalog refresh failed and the stale cache is invalid.",
                    CatalogError::new(
                        "invalid-cached-metadata",
                        "Cached catalog plan does not match the requested plan.",
                    ),
                    error,
                )),
                None => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

async fn load_catalog_live(
    options: &LoadCatalogOptions<'_>,
    endpoints: &super::endpoints::PlanEndpoints,
) -> Result<OpenCodeCatalog, CatalogError> {
    let availability = options
        .http
        .get_json(
            endpoints.models_url,
            true,
            options.api_key,
            options.timeout,
        )
        .await?;
    let available_ids = unique_ids(parse_availability_ids(&availability).map_err(|error| {
        CatalogError::with_cause(
            "invalid-availability",
            "Availability payload failed validation.",
            error,
        )
    })?);

    let metadata = options
        .http
        .get_json(METADATA_URL, false, options.api_key, options.timeout)
        .await?;

    Ok(resolve_catalog(
        options.plan,
        endpoints,
        &available_ids,
        &metadata,
        options.include_deprecated,
        CatalogSource::Network,
    )?)
}

fn resolve_catalog(
    plan_id: OpenCodePlan,
    endpoints: &super::endpoints::PlanEndpoints,
    available_ids: &[String],
    metadata_payload: &Value,
    include_deprecated: bool,
    metadata_source: CatalogSource,
) -> Result<OpenCodeCatalog, CatalogError> {
    let mut diagnostics = Vec::new();
    let mut models = Vec::new();

    let Some(provider_payload) = metadata_payload.get(endpoints.metadata_provider_id) else {
        return Err(CatalogError::new(
            "missing-provider",
            format!(
                "Metadata catalog is missing provider '{}'.",
                endpoints.metadata_provider_id
            ),
        ));
    };

    let provider = parse_metadata_provider(provider_payload).map_err(|error| {
        CatalogError::with_cause(
            "invalid-provider",
            format!(
                "Metadata provider '{}' failed validation.",
                endpoints.metadata_provider_id
            ),
            error,
        )
    })?;

    if provider.id != endpoints.metadata_provider_id {
        return Err(CatalogError::new(
            "invalid-provider",
            format!(
                "Metadata provider id '{}' does not match expected '{}'.",
                provider.id, endpoints.metadata_provider_id
            ),
        ));
    }

    for model_id in available_ids {
        if let Some(model) = resolve_model(
            plan_id,
            endpoints,
            model_id,
            &provider,
            include_deprecated,
            &mut diagnostics,
        ) {
            models.push(model);
        }
    }

    Ok(OpenCodeCatalog {
        plan: plan_id,
        plan_label: endpoints.label.into(),
        metadata_provider_id: endpoints.metadata_provider_id.into(),
        models,
        diagnostics,
        metadata_source,
    })
}

fn resolve_model(
    plan_id: OpenCodePlan,
    endpoints: &super::endpoints::PlanEndpoints,
    model_id: &str,
    provider: &MetadataProvider,
    include_deprecated: bool,
    diagnostics: &mut Vec<CatalogDiagnostic>,
) -> Option<ResolvedModel> {
    let Some(raw) = provider.models.get(model_id) else {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::MissingMetadata,
            model_id: Some(model_id.into()),
            detail: "Available model has no exact metadata match.".into(),
        });
        return None;
    };
    let wire = match parse_wire_model(raw) {
        Ok(model) => model,
        Err(error) => {
            diagnostics.push(CatalogDiagnostic {
                code: DiagnosticCode::InvalidMetadata,
                model_id: Some(model_id.into()),
                detail: error.to_string(),
            });
            return None;
        }
    };

    if wire.id != model_id {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::InvalidMetadata,
            model_id: Some(model_id.into()),
            detail: format!(
                "Metadata id '{}' does not match availability id '{}'.",
                wire.id, model_id
            ),
        });
        return None;
    }

    let status = normalize_status(wire.status);
    if status == ModelStatus::Deprecated && !include_deprecated {
        return None;
    }

    let npm = wire
        .provider
        .as_ref()
        .and_then(|provider| provider.npm.as_deref())
        .or(provider.npm.as_deref());
    let Some(npm) = npm else {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedProtocol,
            model_id: Some(model_id.into()),
            detail: "Metadata does not advertise an AI SDK package.".into(),
        });
        return None;
    };
    let Some(sdk_package) = parse_sdk_package(npm) else {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedProtocol,
            model_id: Some(model_id.into()),
            detail: format!("SDK package '{npm}' is not a supported OpenCode protocol."),
        });
        return None;
    };
    let protocol = sdk_package.protocol();

    let (reasoning_options, option_diagnostics) =
        parse_reasoning_options(wire.reasoning_options.as_deref(), model_id);
    diagnostics.extend(option_diagnostics);

    let reasoning = match derive_reasoning_variants(
        protocol,
        wire.limit.output,
        &reasoning_options,
        model_id,
    ) {
        Ok(plan) => plan,
        Err(error) if error.code == "duplicate-reasoning-variant-id" => {
            diagnostics.push(CatalogDiagnostic {
                code: DiagnosticCode::InvalidMetadata,
                model_id: Some(model_id.into()),
                detail: error.to_string(),
            });
            return None;
        }
        Err(error) => {
            diagnostics.push(CatalogDiagnostic {
                code: DiagnosticCode::InvalidMetadata,
                model_id: Some(model_id.into()),
                detail: error.to_string(),
            });
            return None;
        }
    };
    diagnostics.extend(reasoning.1);

    Some(ResolvedModel {
        plan: plan_id,
        plan_label: endpoints.label.into(),
        metadata_provider_id: endpoints.metadata_provider_id.into(),
        id: wire.id,
        name: wire.name,
        sdk_package,
        protocol,
        context_limit: wire.limit.context,
        explicit_input_limit: wire.limit.input,
        output_limit: wire.limit.output,
        tool_call: wire.tool_call,
        reasoning: wire.reasoning,
        modalities: normalize_modalities(wire.modalities),
        interleaved: normalize_interleaved(wire.interleaved),
        cost: normalize_cost(wire.cost),
        status,
        reasoning_variants: reasoning.0,
    })
}

#[cfg(test)]
fn effective_input_limit(
    context_limit: u64,
    explicit_input_limit: Option<u64>,
    reserved_output_tokens: u64,
) -> Result<u64, CatalogError> {
    if context_limit < reserved_output_tokens {
        return Err(CatalogError::new(
            "negative-effective-input",
            "Reserved output tokens cannot exceed the context limit.",
        ));
    }
    let remaining = context_limit - reserved_output_tokens;
    Ok(explicit_input_limit.unwrap_or(u64::MAX).min(remaining))
}

fn unique_ids(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

fn is_recoverable(error: &CatalogError) -> bool {
    !matches!(error.code, "aborted" | "invalid-catalog")
}

fn with_stale_diagnostic(catalog: OpenCodeCatalog, error: &CatalogError) -> OpenCodeCatalog {
    let mut diagnostics = catalog
        .diagnostics
        .into_iter()
        .filter(|entry| entry.code != DiagnosticCode::StaleMetadata)
        .collect::<Vec<_>>();
    diagnostics.push(CatalogDiagnostic {
        code: DiagnosticCode::StaleMetadata,
        model_id: None,
        detail: format!(
            "Using the last successful {} catalog because refresh failed: {}",
            catalog.plan_label, error
        ),
    });
    OpenCodeCatalog {
        metadata_source: CatalogSource::Cache,
        diagnostics,
        ..catalog
    }
}

#[cfg(test)]
mod tests;
