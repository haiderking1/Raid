use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct TransportError {
    pub code: String,
    message: String,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
    #[source]
    pub cause: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TransportError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            retry_after_ms: None,
            cause: None,
        }
    }

    pub fn with_cause(
        code: impl Into<String>,
        message: impl Into<String>,
        retryable: bool,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable,
            retry_after_ms: None,
            cause: Some(Box::new(cause)),
        }
    }

    pub fn with_retry_after(mut self, retry_after_ms: u64) -> Self {
        self.retry_after_ms = Some(retry_after_ms);
        self
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}
