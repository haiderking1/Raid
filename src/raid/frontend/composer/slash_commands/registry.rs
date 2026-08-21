#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub argument: Option<&'static str>,
}

pub static COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "settings",
        description: "Open settings menu",
        argument: None,
    },
    SlashCommand {
        name: "model",
        description: "<provider/model> - Select model (opens selector UI)",
        argument: Some("<provider/model>"),
    },
    SlashCommand {
        name: "scoped-models",
        description: "Enable/disable models for Ctrl+P cycling",
        argument: None,
    },
    SlashCommand {
        name: "export",
        description: "Export session (HTML default, or specify path: .html/.jsonl)",
        argument: Some("<path>"),
    },
    SlashCommand {
        name: "import",
        description: "Import and resume a session from a JSONL file",
        argument: Some("<path>"),
    },
    SlashCommand {
        name: "clear",
        description: "Clear the current session",
        argument: None,
    },
    SlashCommand {
        name: "connect",
        description: "Connect a provider (interactive API key setup)",
        argument: None,
    },
    SlashCommand {
        name: "compact",
        description: "Compact conversation context",
        argument: None,
    },
    SlashCommand {
        name: "copy",
        description: "Copy the last response",
        argument: None,
    },
    SlashCommand {
        name: "help",
        description: "List slash commands",
        argument: None,
    },
    SlashCommand {
        name: "new",
        description: "Start a new session",
        argument: None,
    },
    SlashCommand {
        name: "resume",
        description: "Resume a previous session",
        argument: Some("<session>"),
    },
    SlashCommand {
        name: "status",
        description: "Show session and model status",
        argument: None,
    },
];
