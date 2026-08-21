mod endpoints;
mod error;
mod json;
mod malformed_tool_call;
mod redact;
mod transport;
mod types;
mod stream_adapter;
mod stream_fn;

#[cfg(test)]
mod cache;
#[cfg(test)]
mod catalog;
#[cfg(test)]
mod reasoning;
#[cfg(test)]
mod validate;
#[cfg(test)]
mod wire;

pub use stream_fn::{convert_agent_messages, opencode_stream_fn, OpenCodeStreamConfig};
pub use types::OpenCodePlan;
