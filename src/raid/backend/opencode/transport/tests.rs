use super::super::reasoning::{derive_reasoning_variants, ReasoningOption};
use super::super::types::{OpenCodeProtocol, ProviderOptions};
use super::usage::FinishReason;
use super::wire_options::openai_compatible_reasoning_effort;

#[test]
fn reasoning_effort_maps_to_openai_compatible_wire_field() {
    let options = ProviderOptions {
        openai_compatible: Some(
            serde_json::json!({ "reasoningEffort": "high" })
                .as_object()
                .unwrap()
                .clone(),
        ),
        ..Default::default()
    };
    assert_eq!(
        openai_compatible_reasoning_effort(&options).as_deref(),
        Some("high")
    );
}

#[test]
fn google_protocol_is_unsupported_without_fetch() {
    let error = super::unsupported_protocol_error("google-generative-ai", Some("secret"));
    assert_eq!(error.code, "unsupported-protocol");
    assert!(!error.message().contains("secret"));
}

#[test]
fn redacts_secret_in_stream_errors() {
    let error = super::TransportError::new(
        "Bearer secret-value failed",
        "network-error",
        true,
    );
    let redacted = super::redact_stream_error(&error, Some("secret-value"));
    assert!(!redacted.message().contains("secret-value"));
}

#[test]
fn anthropic_finish_reason_maps_end_turn_with_tools() {
    assert_eq!(
        super::anthropic_finish_reason(Some("end_turn"), true),
        FinishReason::ToolCalls
    );
}

#[test]
fn derive_budget_variant_for_openai_compatible() {
    let options = vec![ReasoningOption::BudgetTokens {
        min: Some(1024),
        max: Some(8192),
    }];
    let (variants, _) = derive_reasoning_variants(
        OpenCodeProtocol::AnthropicMessages,
        32_000,
        &options,
        "test-model",
    )
    .expect("variants");
    assert!(variants.iter().any(|variant| variant.id.starts_with("budget:")));
}
