use serde_json::{Map, Value};

use super::error::CatalogError;
use super::malformed_tool_call::malformed_tool_call_input;
use super::transport::TransportError;

pub fn read_string(value: &Value) -> Option<&str> {
    value.as_str()
}

pub fn read_finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

pub fn snapshot_safe_json(value: &Value) -> Result<Value, CatalogError> {
    match value {
        Value::Null | Value::Bool(_) => Ok(value.clone()),
        Value::Number(number) => {
            if number.as_f64().is_some_and(|n| n.is_finite()) {
                Ok(value.clone())
            } else {
                Err(json_problem("JSON numbers must be finite."))
            }
        }
        Value::String(text) => Ok(Value::String(text.clone())),
        Value::Array(items) => {
            let mut output = Vec::with_capacity(items.len());
            for item in items {
                output.push(snapshot_safe_json(item)?);
            }
            Ok(Value::Array(output))
        }
        Value::Object(map) => {
            let mut output = Map::new();
            for (key, item) in map {
                output.insert(key.clone(), snapshot_safe_json(item)?);
            }
            Ok(Value::Object(output))
        }
    }
}

#[cfg(test)]
pub fn assert_json_value(value: &Value) -> Result<(), CatalogError> {
    snapshot_safe_json(value).map(|_| ())
}

pub fn parse_json_object(text: &str, code: &'static str) -> Result<Value, TransportError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let parsed: Value = serde_json::from_str(trimmed).map_err(|cause| {
        TransportError::with_cause(
            code,
            "Tool-call arguments were not valid JSON.",
            false,
            cause,
        )
    })?;
    let snapshot = snapshot_safe_json(&parsed).map_err(|cause| {
        TransportError::with_cause(
            code,
            "Tool-call arguments were not valid JSON.",
            false,
            cause,
        )
    })?;
    if !snapshot.is_object() {
        return Err(TransportError::new(
            code,
            "Tool-call arguments were not valid JSON.",
            false,
        ));
    }
    Ok(snapshot)
}

pub fn parse_tool_call_arguments(text: &str) -> Value {
    parse_json_object(text, "malformed-tool-call").unwrap_or_else(|_| malformed_tool_call_input(text))
}

pub fn stringify_json(value: &Value) -> Result<String, TransportError> {
    let safe = snapshot_safe_json(value).map_err(|cause| {
        TransportError::with_cause(
            "invalid-request",
            "Model request contained a value that was not valid JSON.",
            false,
            cause,
        )
    })?;
    serde_json::to_string(&safe).map_err(|cause| {
        TransportError::with_cause(
            "invalid-request",
            "Model request contained a value that was not valid JSON.",
            false,
            cause,
        )
    })
}

fn json_problem(message: impl Into<String>) -> CatalogError {
    CatalogError::new("invalid-json-value", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_nested_json() {
        let value = json!({ "a": [1, { "b": "c" }] });
        assert_eq!(snapshot_safe_json(&value).expect("snapshot"), value);
    }

    #[test]
    fn accepts_finite_numbers() {
        assert_eq!(snapshot_safe_json(&serde_json::json!(1.5)).unwrap(), serde_json::json!(1.5));
    }

    #[test]
    fn parse_tool_call_arguments_falls_back_on_malformed_json() {
        let value = parse_tool_call_arguments("not-json");
        assert_eq!(value["error"], "Tool-call arguments were not valid JSON.");
    }
}
