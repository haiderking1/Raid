use std::time::Duration;

use tokio::runtime::Handle;

use crate::backend::opencode::{
    file_cache, load_catalog, LoadCatalogOptions, MetadataCache, OpenCodeCatalog,
    ReqwestCatalogHttp, ResolvedModel,
};
use crate::config::fuzzy;
use crate::config::{
    provider_by_id, resolve_api_key, AuthStore, ConnectProvider, RaidSettings, PROVIDERS,
};

const LIST_TIMEOUT: Duration = Duration::from_secs(15);
pub const MAX_VISIBLE_MODELS: usize = 8;

pub fn connected_provider() -> Result<&'static ConnectProvider, String> {
    let settings = RaidSettings::load();
    let auth = AuthStore::load();
    let preferred = settings.provider_id();
    if auth.has_provider(preferred) {
        return provider_by_id(preferred)
            .ok_or_else(|| format!("Unknown provider '{preferred}' in settings."));
    }
    PROVIDERS
        .iter()
        .find(|provider| auth.has_provider(provider.id))
        .ok_or_else(|| "No provider connected. Run /connect first.".into())
}

pub fn load_connected_catalog_from_disk(
) -> Result<(OpenCodeCatalog, &'static ConnectProvider), String> {
    let provider = connected_provider()?;
    let catalog = load_provider_catalog_from_disk(provider.id)?;
    Ok((catalog, provider))
}

pub fn load_provider_catalog_from_disk(provider_id: &str) -> Result<OpenCodeCatalog, String> {
    let provider = provider_by_id(provider_id)
        .ok_or_else(|| format!("Unknown provider '{provider_id}'."))?;
    let cache = file_cache();
    cache
        .read(provider.plan)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "No cached model catalog yet.".to_string())
}

pub fn refresh_connected_catalog(runtime: &Handle) -> Result<OpenCodeCatalog, String> {
    runtime
        .block_on(refresh_connected_catalog_async())
        .map_err(|error| error.to_string())
}

pub async fn refresh_connected_catalog_async() -> Result<OpenCodeCatalog, String> {
    let provider = connected_provider()?;
    let api_key = resolve_api_key(provider.id).ok_or_else(|| {
        format!(
            "No API key saved for {}. Run /connect first.",
            provider.label
        )
    })?;
    let http = ReqwestCatalogHttp::default();
    let cache = file_cache();
    load_catalog(LoadCatalogOptions {
        plan: provider.plan,
        api_key: Some(&api_key),
        include_deprecated: false,
        timeout: LIST_TIMEOUT,
        cache: Some(&cache),
        http: &http,
    })
    .await
    .map_err(|error| error.to_string())
}

pub fn filter_model_indices(models: &[ResolvedModel], query: &str) -> Vec<usize> {
    fuzzy::fuzzy_filter_indices_fields(models, query, |model| {
        fuzzy::model_search_fields(&model.id, &model.metadata_provider_id, &model.name).to_vec()
    })
}

pub fn save_default_model(model_id: &str, api: &str) -> Result<(), String> {
    let mut settings = RaidSettings::load();
    settings.default_model = Some(model_id.to_string());
    settings.default_api = Some(api.to_string());
    settings.save()
}

pub fn save_text_generation_model(
    provider_id: &str,
    model_id: &str,
    api: &str,
) -> Result<(), String> {
    let mut settings = RaidSettings::load();
    settings.text_generation_provider = Some(provider_id.to_string());
    settings.text_generation_model = Some(model_id.to_string());
    settings.text_generation_api = Some(api.to_string());
    settings.save()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::opencode::types::{
        CatalogSource, InterleavedFieldState, Modalities, ModelModality, ModelStatus,
        OpenCodePlan, OpenCodeProtocol, ResolvedModel, SdkPackage,
    };
    use crate::config::test_env_lock;
    use std::fs;

    fn sample_model(id: &str, name: &str) -> ResolvedModel {
        ResolvedModel {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            id: id.into(),
            name: name.into(),
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

    fn temp_agent_dir() -> std::path::PathBuf {
        use std::sync::{Mutex, OnceLock};
        static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();
        let counter = COUNTER.get_or_init(|| Mutex::new(0));
        let mut guard = counter.lock().expect("counter lock");
        *guard += 1;
        let id = *guard;
        std::env::temp_dir().join(format!("raid-models-test-{id}"))
    }

    #[test]
    fn filter_model_indices_use_scored_fuzzy_search() {
        let models = vec![
            sample_model("gpt-5.6-luna", "GPT-5.6 Luna"),
            sample_model("glm-5.2", "GLM-5.2"),
            sample_model("deepseek-v4-flash", "DeepSeek V4 Flash"),
            sample_model("kimi-k2.7-code", "Kimi K2.7 Code"),
            sample_model("ox-alpha-free", "Ox Alpha Free"),
            sample_model("qwen3.7-plus", "Qwen3.7 Plus"),
            sample_model("qwen3.6-plus", "Qwen3.6 Plus"),
        ];
        assert_eq!(filter_model_indices(&models, ""), vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(filter_model_indices(&models, "glm"), vec![1]);
        assert_eq!(filter_model_indices(&models, "g56"), vec![0]);
        assert_eq!(filter_model_indices(&models, "flash"), vec![2]);
        assert_eq!(filter_model_indices(&models, "lun"), vec![0]);
        assert_eq!(filter_model_indices(&models, "lu"), vec![0, 5, 6]);
        assert_eq!(
            filter_model_indices(
                &[
                    sample_model("ox-alpha-free", "Ox Alpha Free"),
                    sample_model("minimax-m3", "MiniMax M3"),
                    sample_model("deepseek-v4-flash-vision-exp", "DeepSeek V4 Flash Vision Exp"),
                ],
                "exp",
            ),
            vec![2]
        );
        assert!(filter_model_indices(&models, "zzzz").is_empty());
    }

    #[test]
    fn connected_provider_prefers_settings_when_authenticated() {
        let _guard = test_env_lock();
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }

        let mut auth = AuthStore::default();
        auth.set_api_key("opencode", "zen-key".into());
        auth.set_api_key("opencode-go", "go-key".into());
        auth.save().expect("save auth");

        let mut settings = RaidSettings::load();
        settings.default_provider = Some("opencode".into());
        settings.save().expect("save settings");

        let provider = connected_provider().expect("provider");
        assert_eq!(provider.id, "opencode");

        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }

    #[test]
    fn connected_provider_falls_back_to_any_saved_key() {
        let _guard = test_env_lock();
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }

        let mut auth = AuthStore::default();
        auth.set_api_key("opencode-go", "go-key".into());
        auth.save().expect("save auth");

        let mut settings = RaidSettings::load();
        settings.default_provider = Some("opencode".into());
        settings.save().expect("save settings");

        let provider = connected_provider().expect("provider");
        assert_eq!(provider.id, "opencode-go");

        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }

    #[test]
    fn connected_provider_errors_when_nothing_is_saved() {
        let _guard = test_env_lock();
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }

        assert!(connected_provider().is_err());

        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }

    #[test]
    fn load_connected_catalog_from_disk_reads_saved_cache() {
        let _guard = test_env_lock();
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }

        let mut auth = AuthStore::default();
        auth.set_api_key("opencode-go", "go-key".into());
        auth.save().expect("save auth");

        let catalog = OpenCodeCatalog {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            models: vec![sample_model("alpha", "Alpha")],
            diagnostics: Vec::new(),
            metadata_source: CatalogSource::Cache,
        };
        file_cache()
            .write(OpenCodePlan::Go, &catalog)
            .expect("write cache");

        let (loaded, provider) = load_connected_catalog_from_disk().expect("load cache");
        assert_eq!(provider.id, "opencode-go");
        assert_eq!(loaded.models.len(), 1);
        assert_eq!(loaded.models[0].id, "alpha");

        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }
}
