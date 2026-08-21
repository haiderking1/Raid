use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::paths::{auth_path, write_private_file};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    ApiKey { key: String },
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(flatten)]
    providers: BTreeMap<String, Credential>,
}

impl AuthStore {
    pub fn load() -> Self {
        let path = auth_path();
        if !path.exists() {
            return Self::default();
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => return Self::default(),
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = auth_path();
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        write_private_file(&path, &text).map_err(|error| error.to_string())
    }

    pub fn api_key_for(&self, provider_id: &str) -> Option<String> {
        match self.providers.get(provider_id)? {
            Credential::ApiKey { key } if !key.is_empty() => Some(key.clone()),
            _ => None,
        }
    }

    pub fn set_api_key(&mut self, provider_id: impl Into<String>, key: String) {
        self.providers.insert(
            provider_id.into(),
            Credential::ApiKey { key },
        );
    }

    pub fn remove_provider(&mut self, provider_id: &str) {
        self.providers.remove(provider_id);
    }

    pub fn has_provider(&self, provider_id: &str) -> bool {
        self.api_key_for(provider_id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn temp_agent_dir() -> std::path::PathBuf {
        static COUNTER: OnceLock<Mutex<u64>> = OnceLock::new();
        let counter = COUNTER.get_or_init(|| Mutex::new(0));
        let mut guard = counter.lock().expect("counter lock");
        *guard += 1;
        let id = *guard;
        std::env::temp_dir().join(format!("raid-config-test-{id}"))
    }

    #[test]
    fn round_trips_api_keys() {
        let dir = temp_agent_dir();
        unsafe {
            std::env::set_var("RAID_AGENT_DIR", &dir);
        }
        let mut store = AuthStore::default();
        store.set_api_key("opencode-go", "secret".into());
        store.save().expect("save");
        let loaded = AuthStore::load();
        assert_eq!(loaded.api_key_for("opencode-go"), Some("secret".into()));
        let _ = fs::remove_dir_all(&dir);
        unsafe {
            std::env::remove_var("RAID_AGENT_DIR");
        }
    }
}
