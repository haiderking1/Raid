use super::Role;
use crate::frontend::tools::ToolCall;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineItem {
    Message { role: Role, body: String },
    Tool(ToolCall),
}

impl TimelineItem {
    pub fn message(role: Role, body: String) -> Self {
        Self::Message { role, body }
    }

    pub fn tool(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Tool(ToolCall::running(name, detail))
    }
}
