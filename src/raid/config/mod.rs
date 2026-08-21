mod auth;
mod paths;
mod providers;
mod settings;

pub use auth::AuthStore;
pub use paths::auth_path;
pub use providers::{plan_for_provider_id, ConnectProvider, PROVIDERS};
pub use settings::RaidSettings;

pub fn save_connection(provider_id: &str, api_key: &str) -> Result<(), String> {
    let mut auth = AuthStore::load();
    auth.set_api_key(provider_id, api_key.to_string());
    auth.save()?;

    let mut settings = RaidSettings::load();
    settings.default_provider = Some(provider_id.to_string());
    settings.save()?;
    Ok(())
}

pub fn resolve_api_key(provider_id: &str) -> Option<String> {
    AuthStore::load()
        .api_key_for(provider_id)
        .or_else(|| std::env::var("OPENCODE_API_KEY").ok().filter(|key| !key.is_empty()))
}
