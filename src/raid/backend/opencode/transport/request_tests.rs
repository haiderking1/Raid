use crate::backend::opencode::transport::{
    build_stream_request_body, Message, MessageRole, ModelRequest,
};
use crate::backend::opencode::types::{OpenCodeProtocol, ProviderOptions};

#[test]
fn openai_compatible_stream_body_sets_stream_true() {
    let request = ModelRequest {
        messages: vec![Message {
            role: MessageRole::User,
            content_text: Some("hi".into()),
            assistant_parts: Vec::new(),
            tool_results: Vec::new(),
            provider_metadata: None,
        }],
        tools: Vec::new(),
        provider_options: ProviderOptions::default(),
    };
    let body = build_stream_request_body(
        OpenCodeProtocol::OpenAiCompatible,
        "test-model",
        &request,
        8192,
    )
    .expect("body");
    assert_eq!(body.get("stream").and_then(|value| value.as_bool()), Some(true));
    assert_eq!(
        body.get("model").and_then(|value| value.as_str()),
        Some("test-model")
    );
}

#[test]
fn stream_url_uses_plan_base() {
    use crate::backend::opencode::transport::stream_url;
    use crate::backend::opencode::types::OpenCodePlan;
    assert_eq!(
        stream_url(OpenCodePlan::Go, OpenCodeProtocol::OpenAiCompatible),
        "https://opencode.ai/zen/go/v1/chat/completions"
    );
}
