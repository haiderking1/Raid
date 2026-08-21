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
}

impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "done",
            Self::Failed => "failed",
        }
    }

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
    use super::ToolStatus;

    #[test]
    fn status_labels_match_the_lifecycle() {
        assert_eq!(ToolStatus::Running.label(), "running");
        assert_eq!(ToolStatus::Success.icon(), "✓");
        assert_eq!(ToolStatus::Failed.label(), "failed");
    }
}
