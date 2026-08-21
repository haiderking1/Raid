use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::super::catalog::effective_input_limit;
use super::super::cache::MetadataCache;
use super::super::catalog::{load_catalog, CatalogHttp, LoadCatalogOptions, ReqwestCatalogHttp};
use super::super::endpoints::{metadata_provider_id, plan_endpoints, METADATA_URL, PLAN_IDS};
use super::super::types::{OpenCodePlan, SdkPackage};

#[test]
fn plan_ids_cover_supported_plans() {
    assert_eq!(PLAN_IDS, [OpenCodePlan::Zen, OpenCodePlan::Go]);
    assert_eq!(metadata_provider_id(OpenCodePlan::Zen), "opencode");
    assert_eq!(metadata_provider_id(OpenCodePlan::Go), "opencode-go");
}

#[test]
fn effective_input_limit_reserves_output_tokens() {
    assert_eq!(effective_input_limit(32_000, None, 8_192).expect("limit"), 23_808);
    assert!(effective_input_limit(4_096, None, 8_192).is_err());
}

struct MockHttp {
    responses: Mutex<HashMap<String, Value>>,
}

impl MockHttp {
    fn new(responses: HashMap<String, Value>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl CatalogHttp for MockHttp {
    async fn get_json(
        &self,
        url: &str,
        _authorize: bool,
        _api_key: Option<&str>,
        _timeout: Duration,
    ) -> Result<Value, super::super::error::CatalogError> {
        self.responses
            .lock()
            .expect("mock lock")
            .get(url)
            .cloned()
            .ok_or_else(|| {
                super::super::error::CatalogError::new("fetch-failed", format!("No mock for {url}"))
            })
    }
}

fn base_model(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "release_date": "2026-01-01",
        "attachment": true,
        "reasoning": true,
        "temperature": false,
        "tool_call": true,
        "limit": {
            "context": 200000,
            "input": 176000,
            "output": 32000
        },
        "modalities": {
            "input": ["text"],
            "output": ["text"]
        }
    })
}

fn model_with_provider(id: &str, name: &str, npm: &str) -> Value {
    let mut model = base_model(id, name);
    model
        .as_object_mut()
        .expect("object")
        .insert("provider".into(), json!({ "npm": npm }));
    model
}

fn metadata_catalog(models: Value) -> Value {
    json!({
        "opencode": {
            "id": "opencode",
            "name": "OpenCode Zen",
            "npm": "@ai-sdk/openai-compatible",
            "models": models.get("opencode").cloned().unwrap_or(json!({}))
        },
        "opencode-go": {
            "id": "opencode-go",
            "name": "OpenCode Go",
            "npm": "@ai-sdk/openai-compatible",
            "models": models.get("opencode-go").cloned().unwrap_or(json!({}))
        }
    })
}

#[tokio::test]
async fn go_catalog_uses_go_endpoint_and_metadata_provider() {
    let go = plan_endpoints(OpenCodePlan::Go);
    let zen = plan_endpoints(OpenCodePlan::Zen);
    let http = MockHttp::new(HashMap::from([
        (
            go.models_url.into(),
            json!({
                "object": "list",
                "data": [{ "id": "kimi-k3" }]
            }),
        ),
        (
            zen.models_url.into(),
            json!({
                "object": "list",
                "data": [{ "id": "gpt-5.5" }]
            }),
        ),
        (
            METADATA_URL.into(),
            metadata_catalog(json!({
                "opencode-go": {
                    "kimi-k3": model_with_provider("kimi-k3", "Kimi K3", "@ai-sdk/openai-compatible")
                }
            })),
        ),
    ]));

    let catalog = load_catalog(LoadCatalogOptions {
        plan: OpenCodePlan::Go,
        api_key: Some("test-key"),
        include_deprecated: false,
        timeout: Duration::from_secs(5),
        cache: None,
        http: &http,
    })
    .await
    .expect("catalog");

    assert_eq!(catalog.plan, OpenCodePlan::Go);
    assert_eq!(catalog.plan_label, "Go");
    assert_eq!(catalog.metadata_provider_id, "opencode-go");
    assert_eq!(catalog.models.len(), 1);
    assert_eq!(catalog.models[0].id, "kimi-k3");
    assert_eq!(catalog.models[0].sdk_package, SdkPackage::OpenAiCompatible);
}

#[tokio::test]
async fn missing_metadata_is_diagnosed_not_invented() {
    let go = plan_endpoints(OpenCodePlan::Go);
    let http = MockHttp::new(HashMap::from([
        (
            go.models_url.into(),
            json!({
                "object": "list",
                "data": [{ "id": "ghost-model" }]
            }),
        ),
        (
            METADATA_URL.into(),
            metadata_catalog(json!({
                "opencode-go": {}
            })),
        ),
    ]));

    let catalog = load_catalog(LoadCatalogOptions {
        plan: OpenCodePlan::Go,
        api_key: Some("test-key"),
        include_deprecated: false,
        timeout: Duration::from_secs(5),
        cache: None,
        http: &http,
    })
    .await
    .expect("catalog");

    assert!(catalog.models.is_empty());
    assert!(catalog
        .diagnostics
        .iter()
        .any(|entry| entry.model_id.as_deref() == Some("ghost-model")));
}

#[tokio::test]
async fn stale_cache_is_used_when_refresh_fails() {
    let go = plan_endpoints(OpenCodePlan::Go);
    let cache = super::super::cache::memory_cache();
    let seeded = load_catalog(LoadCatalogOptions {
        plan: OpenCodePlan::Go,
        api_key: Some("test-key"),
        include_deprecated: false,
        timeout: Duration::from_secs(5),
        cache: Some(&cache),
        http: &MockHttp::new(HashMap::from([
            (
                go.models_url.into(),
                json!({
                    "object": "list",
                    "data": [{ "id": "kimi-k3" }]
                }),
            ),
            (
                METADATA_URL.into(),
                metadata_catalog(json!({
                    "opencode-go": {
                        "kimi-k3": model_with_provider("kimi-k3", "Kimi K3", "@ai-sdk/openai-compatible")
                    }
                })),
            ),
        ])),
    })
    .await
    .expect("seed");

    cache.write(OpenCodePlan::Go, &seeded).expect("write");

    let broken = MockHttp::new(HashMap::from([(go.models_url.into(), json!({"bad": true}))]));
    let catalog = load_catalog(LoadCatalogOptions {
        plan: OpenCodePlan::Go,
        api_key: Some("test-key"),
        include_deprecated: false,
        timeout: Duration::from_secs(5),
        cache: Some(&cache),
        http: &broken,
    })
    .await
    .expect("stale fallback");

    assert_eq!(catalog.models.len(), 1);
    assert!(catalog
        .diagnostics
        .iter()
        .any(|entry| matches!(entry.code, super::super::types::DiagnosticCode::StaleMetadata)));
}

#[cfg(test)]
mod live {
    use super::*;

    #[tokio::test]
    #[ignore = "hits live OpenCode endpoints; run with OPENCODE_LIVE_CATALOG=1"]
    async fn live_go_catalog_intersects_official_metadata() {
        if std::env::var("OPENCODE_LIVE_CATALOG").ok().as_deref() != Some("1") {
            return;
        }
        let api_key = std::env::var("OPENCODE_API_KEY").ok();
        let http = ReqwestCatalogHttp::default();
        let catalog = load_catalog(LoadCatalogOptions {
            plan: OpenCodePlan::Go,
            api_key: api_key.as_deref(),
            include_deprecated: false,
            timeout: Duration::from_secs(20),
            cache: None,
            http: &http,
        })
        .await
        .expect("live go catalog");

        assert_eq!(catalog.metadata_provider_id, "opencode-go");
        assert!(!catalog.models.is_empty());
        for model in &catalog.models {
            assert_eq!(model.plan, OpenCodePlan::Go);
            assert!(!model.reasoning_variants.is_empty());
        }
    }
}
