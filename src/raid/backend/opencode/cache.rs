use std::collections::HashMap;
use std::sync::Mutex;

use super::types::{OpenCodeCatalog, OpenCodePlan};
use super::validate::validate_catalog_invariants;

pub trait MetadataCache: Send + Sync {
    fn read(&self, plan: OpenCodePlan) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError>;
    fn write(&self, plan: OpenCodePlan, catalog: &OpenCodeCatalog) -> Result<(), super::error::CatalogError>;
}

pub fn memory_cache() -> MemoryMetadataCache {
    MemoryMetadataCache::default()
}

#[derive(Default)]
pub struct MemoryMetadataCache {
    values: Mutex<HashMap<OpenCodePlan, OpenCodeCatalog>>,
}

impl MetadataCache for MemoryMetadataCache {
    fn read(&self, plan: OpenCodePlan) -> Result<Option<OpenCodeCatalog>, super::error::CatalogError> {
        let catalog = self.values.lock().expect("cache lock").get(&plan).cloned();
        if let Some(catalog) = &catalog {
            validate_catalog_invariants(catalog)?;
        }
        Ok(catalog)
    }

    fn write(
        &self,
        plan: OpenCodePlan,
        catalog: &OpenCodeCatalog,
    ) -> Result<(), super::error::CatalogError> {
        validate_catalog_invariants(catalog)?;
        self.values
            .lock()
            .expect("cache lock")
            .insert(plan, catalog.clone());
        Ok(())
    }
}
