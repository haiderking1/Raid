use thiserror::Error;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CatalogError {
    pub code: &'static str,
    message: String,
    #[source]
    pub cause: Option<Box<dyn std::error::Error + Send + Sync>>,
    pub refresh_error: Option<Box<CatalogError>>,
}

impl CatalogError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            cause: None,
            refresh_error: None,
        }
    }

    pub fn with_cause(
        code: &'static str,
        message: impl Into<String>,
        cause: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            cause: Some(Box::new(cause)),
            refresh_error: None,
        }
    }

    pub fn stale_cache(
        message: impl Into<String>,
        cause: CatalogError,
        refresh: CatalogError,
    ) -> Self {
        Self {
            code: "invalid-cached-metadata",
            message: message.into(),
            cause: Some(Box::new(cause)),
            refresh_error: Some(Box::new(refresh)),
        }
    }
}
