#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
    #[cfg_attr(not(test), expect(dead_code))]
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    pub detail: String,
    pub status: ToolStatus,
    pub summary: String,
}

impl ToolCall {
    pub fn running(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
            status: ToolStatus::Running,
            summary: String::from("Running"),
        }
    }

    pub fn finish(&mut self, status: ToolStatus, summary: impl Into<String>) {
        self.status = status;
        self.summary = summary.into();
    }

    #[cfg(test)]
    pub fn finished(mut self, status: ToolStatus, summary: impl Into<String>) -> Self {
        self.finish(status, summary);
        self
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn new(name: impl Into<String>, detail: impl Into<String>, status: ToolStatus) -> Self {
        let name = name.into();
        let detail = detail.into();
        let summary = default_summary(&name, status);
        Self {
            name,
            detail,
            status,
            summary,
        }
    }

    pub fn display_name(&self) -> String {
        let mut chars = self.name.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        }
    }
}

#[cfg_attr(not(test), expect(dead_code))]
fn default_summary(name: &str, status: ToolStatus) -> String {
    match status {
        ToolStatus::Running => String::from("Running"),
        ToolStatus::Failed => String::from("Failed"),
        ToolStatus::Success => {
            let mut chars = name.chars();
            let title = match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            };
            format!("{title} (ctrl+r to expand)")
        }
    }
}

impl ToolStatus {
    #[cfg_attr(not(test), expect(dead_code))]
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "done",
            Self::Failed => "failed",
        }
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn icon(self) -> &'static str {
        match self {
            Self::Running => "▸",
            Self::Success => "✓",
            Self::Failed => "!",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolCall, ToolStatus};

    #[test]
    fn status_labels_match_the_lifecycle() {
        assert_eq!(ToolStatus::Running.label(), "running");
        assert_eq!(ToolStatus::Success.icon(), "✓");
        assert_eq!(ToolStatus::Failed.label(), "failed");
    }

    #[test]
    fn display_name_title_cases_the_tool() {
        assert_eq!(ToolCall::running("read", "src/main.rs").display_name(), "Read");
        assert_eq!(
            ToolCall::running("bash", "cargo test").display_name(),
            "Bash"
        );
    }
}
