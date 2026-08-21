mod cache;
mod catalog;
mod endpoints;
mod error;
mod json;
mod malformed_tool_call;
mod reasoning;
mod redact;
mod transport;
pub mod types;
mod validate;
mod wire;
mod stream_adapter;
mod stream_fn;

pub use cache::{file_cache, MetadataCache};
pub use catalog::{load_catalog, LoadCatalogOptions, ReqwestCatalogHttp};
pub use stream_fn::{convert_agent_messages, opencode_stream_fn, OpenCodeStreamConfig};
pub use types::{OpenCodeCatalog, OpenCodePlan, ResolvedModel};
