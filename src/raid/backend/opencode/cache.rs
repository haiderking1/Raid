use std::fs;
use std::path::Path;

use serde_json::Value;

use super::types::{OpenCodeCatalog, OpenCodePlan};
use super::validate::{parse_cached_catalog, serialize_catalog};

pub trait MetadataCache: Send + Sync {
    fn read(&self, plan: OpenCodePlan) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError>;
    fn write(&self, plan: OpenCodePlan, catalog: &OpenCodeCatalog) -> Result<(), super::error::CatalogError>;
}

#[cfg(test)]
pub fn memory_cache() -> MemoryMetadataCache {
    MemoryMetadataCache::default()
}

pub fn file_cache() -> FileMetadataCache {
    FileMetadataCache
}

#[cfg(test)]
#[derive(Default)]
pub struct MemoryMetadataCache {
    values: std::sync::Mutex<std::collections::HashMap<OpenCodePlan, OpenCodeCatalog>>,
}

#[cfg(test)]
impl MetadataCache for MemoryMetadataCache {
    fn read(&self, plan: OpenCodePlan) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError> {
        let catalog = self.values.lock().expect("cache lock").get(&plan).cloned();
        if let Some(catalog) = &catalog {
            super::validate::validate_catalog_invariants(catalog)?;
        }
        Ok(catalog)
    }

    fn write(
        &self,
        plan: OpenCodePlan,
        catalog: &OpenCodeCatalog,
    ) -> Result<(), super::error::CatalogError> {
        super::validate::validate_catalog_invariants(catalog)?;
        self.values
            .lock()
            .expect("cache lock")
            .insert(plan, catalog.clone());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FileMetadataCache;

impl FileMetadataCache {
    pub fn cache_path(plan: OpenCodePlan) -> std::path::PathBuf {
        crate::config::catalog_cache_path(plan_slug(plan))
    }
}

impl MetadataCache for FileMetadataCache {
    fn read(&self, plan: OpenCodePlan) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError> {
        let path = Self::cache_path(plan);
        if !path.exists() {
            return Ok(None);
        }
        read_catalog_file(&path)
    }

    fn write(
        &self,
        plan: OpenCodePlan,
        catalog: &OpenCodeCatalog,
    ) -> Result<(), super::error::CatalogError> {
        let path = Self::cache_path(plan);
        write_catalog_file(&path, catalog)
    }
}

fn plan_slug(plan: OpenCodePlan) -> &'static str {
    match plan {
        OpenCodePlan::Zen => "zen",
        OpenCodePlan::Go => "go",
    }
}

fn read_catalog_file(path: &Path) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError> {
    let text = fs::read_to_string(path).map_err(|error| {
        super::error::CatalogError::with_cause(
            "invalid-cached-metadata",
            format!("Failed to read cached catalog at {}.", path.display()),
            error,
        )
    })?;
    let payload: Value = serde_json::from_str(&text).map_err(|error| {
        super::error::CatalogError::with_cause(
            "invalid-cached-metadata",
            format!("Cached catalog at {} was not valid JSON.", path.display()),
            error,
        )
    })?;
    Ok(Some(parse_cached_catalog(&payload)?))
}

fn write_catalog_file(path: &Path, catalog: &OpenCodeCatalog) -> Result<(), super::error::CatalogError> {
    let payload = serialize_catalog(catalog)?;
    let text = serde_json::to_string_pretty(&payload).map_err(|error| {
        super::error::CatalogError::with_cause(
            "invalid-catalog",
            "Cached catalog failed serialization.",
            error,
        )
    })?;
    crate::config::write_private_file(path, &text).map_err(|error| {
        super::error::CatalogError::with_cause(
            "invalid-cached-metadata",
            format!("Failed to write cached catalog to {}.", path.display()),
            error,
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::opencode::types::{
        CatalogSource, InterleavedFieldState, Modalities, ModelModality, ModelStatus,
        OpenCodeProtocol, ResolvedModel, SdkPackage,
    };
    use crate::config::test_env_lock;
    use std::fs;

    fn sample_catalog() -> OpenCodeCatalog {
        OpenCodeCatalog {
            plan: OpenCodePlan::Go,
            plan_label: "Go".into(),
            metadata_provider_id: "opencode-go".into(),
            models: vec![ResolvedModel {
                plan: OpenCodePlan::Go,
                plan_label: "Go".into(),
                metadata_provider_id: "opencode-go".into(),
                id: "kimi-k3".into(),
                name: "Kimi K3".into(),
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
            }],
            diagnostics: Vec::new(),
            metadata_source: CatalogSource::Network,
        }
    }

    fn temp_agent_dir() -> std::path::PathBuf {
        use std::sync::{Mutex, OnceLock};
        static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();
        let counter = COUNTER.get_or_init(|| Mutex::new(0));
        let mut guard = counter.lock().expect("counter lock");
        *guard += 1;
        let id = *guard;
        std::env::temp_dir().join(format!("raid-cache-test-{id}"))
    }

    #[test]
    fn file_cache_round_trips_catalog_to_disk() {
        let _guard = test_env_lock();
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }

        let cache = file_cache();
        let catalog = sample_catalog();
        cache
            .write(OpenCodePlan::Go, &catalog)
            .expect("write cache");
        let loaded = cache
            .read(OpenCodePlan::Go)
            .expect("read cache")
            .expect("cached catalog");
        assert_eq!(loaded, catalog);
        assert!(FileMetadataCache::cache_path(OpenCodePlan::Go).exists());

        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }
}
