pub mod agent_loop;
pub mod event_stream;
pub mod stream_fn;
pub mod types;
pub mod validation;

pub use agent_loop::{agent_loop, AgentLoopHandle};
#[cfg(test)]
pub use agent_loop::agent_loop_continue;
pub use event_stream::{assistant_message_stream, AssistantMessageStream};
pub use stream_fn::{get_default_stream_fn, set_default_stream_fn};
pub use types::*;

#[cfg(test)]
mod tests;
