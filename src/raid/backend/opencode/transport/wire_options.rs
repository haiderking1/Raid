use serde_json::{Map, Value};

use super::super::types::ProviderOptions;

pub fn openai_responses_reasoning(provider_options: &ProviderOptions) -> Option<Map<String, Value>> {
    let openai = provider_options.openai.as_ref()?;
    let effort = openai.get("reasoningEffort")?.as_str()?;
    let mut map = Map::new();
    map.insert("effort".into(), Value::String(effort.to_string()));
    Some(map)
}

pub fn openai_compatible_reasoning_effort(provider_options: &ProviderOptions) -> Option<String> {
    provider_options
        .openai_compatible
        .as_ref()?
        .get("reasoningEffort")?
        .as_str()
        .map(str::to_string)
}

pub fn anthropic_thinking(provider_options: &ProviderOptions) -> Option<Map<String, Value>> {
    let anthropic = provider_options.anthropic.as_ref()?;
    let thinking = anthropic.get("thinking")?;
    let record = thinking.as_object()?;
    let thinking_type = record.get("type")?.as_str()?;
    let mut map = Map::new();
    map.insert("type".into(), Value::String(thinking_type.to_string()));
    if let Some(budget) = record.get("budgetTokens").and_then(|value| value.as_i64()) {
        if budget > 0 {
            map.insert("budget_tokens".into(), Value::Number(budget.into()));
        }
    }
    Some(map)
}

pub fn anthropic_effort(provider_options: &ProviderOptions) -> Option<String> {
    provider_options
        .anthropic
        .as_ref()?
        .get("effort")?
        .as_str()
        .map(str::to_string)
}

fn positive_safe_integer(value: &Value) -> Option<u64> {
    value
        .as_i64()
        .filter(|number| *number > 0 && (*number as u64) <= i64::MAX as u64)
        .map(|number| number as u64)
}

pub fn anthropic_max_tokens(provider_options: &ProviderOptions) -> Option<u64> {
    provider_options
        .anthropic
        .as_ref()?
        .get("maxTokens")
        .and_then(positive_safe_integer)
}

pub fn openai_compatible_max_tokens(provider_options: &ProviderOptions) -> Option<u64> {
    provider_options
        .openai_compatible
        .as_ref()?
        .get("maxTokens")
        .and_then(positive_safe_integer)
}

pub fn openai_responses_max_output_tokens(provider_options: &ProviderOptions) -> Option<u64> {
    provider_options
        .openai
        .as_ref()?
        .get("maxOutputTokens")
        .and_then(positive_safe_integer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_openai_compatible_reasoning_effort() {
        let options = ProviderOptions {
            openai_compatible: Some(json!({ "reasoningEffort": "high" }).as_object().unwrap().clone()),
            ..Default::default()
        };
        assert_eq!(
            openai_compatible_reasoning_effort(&options).as_deref(),
            Some("high")
        );
    }
}
