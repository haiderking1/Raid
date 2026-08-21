use std::collections::HashSet;

use serde_json::{Map, Value};

pub const REDACTED_PLACEHOLDER: &str = "[redacted]";
pub const CIRCULAR_PLACEHOLDER: &str = "[Circular]";
pub const UNREADABLE_PLACEHOLDER: &str = "[Unreadable]";

pub fn redact_secret(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|value| !value.is_empty()) else {
        return text.to_string();
    };
    text.replace(secret, REDACTED_PLACEHOLDER)
}

pub fn redact_unknown(value: &Value, secret: Option<&str>) -> Value {
    redact_value(value, secret, &mut HashSet::new())
}

pub fn redact_error(error: &(dyn std::error::Error + 'static), secret: Option<&str>) -> String {
    redact_secret(&error.to_string(), secret)
}

fn redact_value(value: &Value, secret: Option<&str>, seen: &mut HashSet<usize>) -> Value {
    match value {
        Value::Null | Value::Bool(_) => value.clone(),
        Value::Number(number) => Value::Number(number.clone()),
        Value::String(text) => Value::String(redact_secret(text, secret)),
        Value::Array(items) => {
            let pointer = items.as_ptr() as usize;
            if !seen.insert(pointer) {
                return Value::String(CIRCULAR_PLACEHOLDER.into());
            }
            let output = items
                .iter()
                .map(|item| redact_value(item, secret, seen))
                .collect();
            seen.remove(&pointer);
            Value::Array(output)
        }
        Value::Object(map) => {
            let mut output = Map::new();
            for (key, item) in map {
                output.insert(
                    redact_secret(key, secret),
                    redact_value(item, secret, seen),
                );
            }
            Value::Object(output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "sk-secret-key";

    #[test]
    fn redacts_secret_in_strings() {
        assert_eq!(
            redact_secret("Bearer sk-secret-key", Some(SECRET)),
            format!("Bearer {REDACTED_PLACEHOLDER}")
        );
    }

    #[test]
    fn redacts_nested_values() {
        let value = json!({ "token": "sk-secret-key", "nested": ["sk-secret-key"] });
        let redacted = redact_unknown(&value, Some(SECRET));
        assert_eq!(redacted["token"], REDACTED_PLACEHOLDER);
        assert_eq!(redacted["nested"][0], REDACTED_PLACEHOLDER);
    }
}
