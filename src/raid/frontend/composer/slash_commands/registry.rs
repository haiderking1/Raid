#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
    pub argument: Option<&'static str>,
}

pub static COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "connect",
        description: "Connect a provider",
        argument: None,
    },
    SlashCommand {
        name: "model",
        description: "Select a model",
        argument: None,
    },
    SlashCommand {
        name: "new",
        description: "Start a new session",
        argument: None,
    },
    SlashCommand {
        name: "resume",
        description: "Resume a different session",
        argument: None,
    },
];
