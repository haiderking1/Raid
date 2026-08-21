use std::fs;

use serde::{Deserialize, Serialize};

use super::paths::{settings_path, write_private_file};

pub const DEFAULT_PROVIDER: &str = "opencode-go";
pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaidSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_generation_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_generation_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_generation_api: Option<String>,
}

impl Default for RaidSettings {
    fn default() -> Self {
        Self {
            default_provider: Some(DEFAULT_PROVIDER.into()),
            default_model: Some(DEFAULT_MODEL.into()),
            default_api: Some("openai-compatible".into()),
            text_generation_provider: None,
            text_generation_model: None,
            text_generation_api: None,
        }
    }
}

impl RaidSettings {
    pub fn load() -> Self {
        let path = settings_path();
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
        let path = settings_path();
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        write_private_file(&path, &text).map_err(|error| error.to_string())
    }

    pub fn provider_id(&self) -> &str {
        self.default_provider
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_PROVIDER)
    }

    pub fn model_id(&self) -> &str {
        self.default_model
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_MODEL)
    }

    pub fn api(&self) -> &str {
        self.default_api
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("openai-compatible")
    }

    pub fn text_generation_provider_id(&self) -> &str {
        self.text_generation_provider
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.provider_id())
    }

    pub fn text_generation_model_id(&self) -> &str {
        self.text_generation_model
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.model_id())
    }

    pub fn text_generation_api(&self) -> &str {
        self.text_generation_api
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| self.api())
    }
}

#[cfg(test)]
mod tests {
    use super::RaidSettings;

    #[test]
    fn text_generation_model_falls_back_to_chat_model() {
        let settings: RaidSettings = serde_json::from_str(
            r#"{"default_provider":"provider-a","default_model":"chat-a","default_api":"responses"}"#,
        )
        .expect("settings");

        assert_eq!(settings.text_generation_provider_id(), "provider-a");
        assert_eq!(settings.text_generation_model_id(), "chat-a");
        assert_eq!(settings.text_generation_api(), "responses");
    }

    #[test]
    fn text_generation_model_can_be_selected_independently() {
        let settings: RaidSettings = serde_json::from_str(
            r#"{
                "default_provider":"provider-a",
                "default_model":"chat-a",
                "default_api":"responses",
                "text_generation_provider":"provider-b",
                "text_generation_model":"text-b",
                "text_generation_api":"openai-compatible"
            }"#,
        )
        .expect("settings");

        assert_eq!(settings.text_generation_provider_id(), "provider-b");
        assert_eq!(settings.text_generation_model_id(), "text-b");
        assert_eq!(settings.text_generation_api(), "openai-compatible");
    }
}
