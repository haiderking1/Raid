# OpenCode provider (Raid)

Native OpenCode catalog loading and protocol transport handlers for Raid.

| Source | URL | Role |
|--------|-----|------|
| Availability | `https://opencode.ai/zen/go/v1/models` (Go) | Which model IDs exist |
| Metadata | `https://models.opencode.ai/api.json` | Limits, SDK protocol, reasoning, cost |

Go models live under metadata provider `opencode-go`. Zen uses `opencode` at `https://opencode.ai/zen/v1/models`.

## Catalog

```rust
use crate::backend::opencode::{
    load_catalog, memory_cache, CatalogHttp, LoadCatalogOptions, OpenCodePlan, ReqwestCatalogHttp,
};

let http = ReqwestCatalogHttp::default();
let cache = memory_cache();
let catalog = load_catalog(LoadCatalogOptions {
    plan: OpenCodePlan::Go,
    api_key: Some(&std::env::var("OPENCODE_API_KEY").expect("key")),
    include_deprecated: false,
    timeout: std::time::Duration::from_secs(10),
    cache: Some(&cache),
    http: &http,
})
.await?;
```

Cached catalogs are validated on read/write via `parse_cached_catalog` / `serialize_catalog`.
Successful loads are persisted under `~/.raid/agent/catalog-{zen,go}.json` and reused on the next launch.
The model picker reads the disk cache immediately and refreshes from the API in the background.

## Transports (handler layer)

Protocol SSE handlers are testable without the TUI:

- `process_openai_compatible_events`
- `process_openai_responses_events`
- `process_anthropic_messages_events`
- `process_protocol_events` (dispatch + Google → `unsupported-protocol`)

Supporting modules: `messages`, `wire_options`, `usage`, `complete_tool_call`, `http`, `sse`, `json`, `redact`.

## Live catalog check

```bash
OPENCODE_LIVE_CATALOG=1 OPENCODE_API_KEY=... cargo test live_go_catalog
```
