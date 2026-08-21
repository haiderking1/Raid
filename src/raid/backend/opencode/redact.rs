pub const REDACTED_PLACEHOLDER: &str = "[redacted]";

pub fn redact_secret(text: &str, secret: Option<&str>) -> String {
    let Some(secret) = secret.filter(|value| !value.is_empty()) else {
        return text.to_string();
    };
    text.replace(secret, REDACTED_PLACEHOLDER)
}

pub fn redact_error(error: &(dyn std::error::Error + 'static), secret: Option<&str>) -> String {
    redact_secret(&error.to_string(), secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "sk-secret-key";

    #[test]
    fn redacts_secret_in_strings() {
        assert_eq!(
            redact_secret("Bearer sk-secret-key", Some(SECRET)),
            format!("Bearer {REDACTED_PLACEHOLDER}")
        );
    }
}
