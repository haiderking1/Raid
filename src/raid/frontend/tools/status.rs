#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
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

    pub fn is_expandable(&self) -> bool {
        self.status != ToolStatus::Running
            && (self.summary.lines().count() > 1 || self.summary.chars().count() > 80)
    }

    pub fn compact_summary(&self) -> String {
        if self.status == ToolStatus::Running {
            return String::from("Running");
        }
        if !self.is_expandable() {
            return self.summary.clone();
        }

        let label = if self.status == ToolStatus::Failed {
            String::from("Failed")
        } else {
            let line_count = self.summary.lines().count();
            if line_count > 1 {
                format!("{} {line_count} lines", self.display_name())
            } else {
                format!("{} output", self.display_name())
            }
        };
        format!("{label} (ctrl+o to expand)")
    }
}

fn default_summary(name: &str, status: ToolStatus) -> String {
    match status {
        ToolStatus::Running => String::from("Running"),
        ToolStatus::Failed => String::from("Failed"),
        ToolStatus::Success => {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        }
    }
}

impl ToolStatus {
    #[cfg(test)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "done",
            Self::Failed => "failed",
        }
    }

    #[cfg(test)]
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
        assert_eq!(
            ToolCall::running("read", "src/main.rs").display_name(),
            "Read"
        );
        assert_eq!(
            ToolCall::running("bash", "cargo test").display_name(),
            "Bash"
        );
    }

    #[test]
    fn long_results_collapse_behind_ctrl_o() {
        let call = ToolCall::running("read", "src/main.rs")
            .finished(ToolStatus::Success, "one\ntwo\nthree");
        assert!(call.is_expandable());
        assert_eq!(call.compact_summary(), "Read 3 lines (ctrl+o to expand)");
    }

    #[test]
    fn short_results_stay_visible() {
        let call = ToolCall::running("write", "src/main.rs")
            .finished(ToolStatus::Success, "Successfully wrote 12 bytes");
        assert!(!call.is_expandable());
        assert_eq!(call.compact_summary(), "Successfully wrote 12 bytes");
    }
}
