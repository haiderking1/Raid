#[derive(Debug, PartialEq, Eq)]
pub enum ComposerAction {
    None,
    Submit(String),
    Command { name: String, args: String },
    Quit,
}
