mod bash;
mod env;
mod file_mutation_queue;
mod image;
mod path_utils;
mod read;
mod shell_output;
mod truncate;
mod write;

use std::sync::Arc;

pub use bash::BashTool;
pub use env::ToolEnvironment;
pub use read::ReadTool;
pub use write::WriteTool;

pub fn default_tools(env: Arc<ToolEnvironment>) -> Vec<Arc<dyn crate::backend::agent::AgentTool>> {
    vec![
        Arc::new(BashTool::new(env.clone())),
        Arc::new(ReadTool::new(env.clone())),
        Arc::new(WriteTool::new(env)),
    ]
}

#[cfg(test)]
mod tests;
