# Raid

Raid is a terminal coding agent written in Rust. Start it inside a project and it can inspect the workspace, stream model responses, run shell commands, read files and images, and write files.

Raid is still early. It can execute arbitrary commands and overwrite files, so use it in a project you can recover with Git.

## What works

- Streaming Markdown responses with live reasoning and activity status
- Bash, file reading, image reading, and file writing tools
- Collapsed tool cards with live command output and expandable details
- Fuzzy model search across connected providers
- A separate model choice for short text generation, currently used for session titles
- One SQLite database per session, grouped by project
- Generated session titles based only on the first user message
- Live session search, resume, locking, and recoverable deletion through the system trash
- Automatic context compaction and manual compaction with an optional focus
- A persistent status line for the model, context usage, thinking level, and workspace path
- Mouse scrolling at three lines per wheel tick

## Requirements

- Rust nightly, configured by [`rust-toolchain.toml`](rust-toolchain.toml)
- Git
- An API key for OpenCode Zen or OpenCode Go

## Build and run

```bash
git clone https://github.com/haiderking1/Raid.git
cd Raid
cargo build --release
./target/release/raid
```

To run `raid` from any directory while keeping it linked to this checkout:

```bash
mkdir -p "$HOME/.local/bin"
ln -sfn "$(pwd)/target/release/raid" "$HOME/.local/bin/raid"
```

Make sure `~/.local/bin` is on your `PATH`. Add this to your shell configuration if needed:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

After pulling an update, rebuild the binary. The symlink does not need to be recreated.

```bash
git pull
cargo build --release
```

Then enter any project and launch Raid:

```bash
cd /path/to/project
raid
```

## First setup

Run these commands inside Raid:

1. `/connect` to choose a provider and save its API key.
2. `/model` to choose the chat and coding model.
3. `/text-model` if you want a different model to name sessions.

Raid saves the selected models and uses them on the next launch.

## Launch options

| Command | Action |
| --- | --- |
| `raid` | Start a new session for the current project |
| `raid -c` | Continue the most recently used session for the current project |
| `raid -r` | Open the session picker at startup |
| `raid --no-session` | Run without creating or writing session files |
| `raid --session PATH` | Open a specific session database |
| `raid --help` | Show all command-line options |

## Slash commands

| Command | Action |
| --- | --- |
| `/connect` | Connect a provider |
| `/model` | Select the chat and coding model |
| `/text-model` | Select the model used to generate session titles |
| `/compact [focus]` | Summarize older context, optionally with a requested focus |
| `/new` | Start a new session |
| `/resume` | Search and resume a saved session for the current project |

Type `/` to open the command palette. Use the arrow keys to move, `Tab` to complete a command, and `Enter` to run it.

## Keybindings

| Key | Action |
| --- | --- |
| `Enter` | Submit the composer or confirm the selected item |
| `Shift+Enter` | Insert a newline |
| `Esc` | Interrupt the active response or close the current panel |
| `Ctrl+C` | Clear the draft, or quit when the composer is empty |
| `Ctrl+N` | Start a new session |
| `Ctrl+R` | Open the session picker |
| `Ctrl+O` | Expand or collapse tool details |
| `PageUp` / `PageDown` | Scroll the conversation by one page |
| Mouse wheel | Scroll the conversation by three lines |

Inside the session picker, type to filter by title or session ID. Press `Ctrl+D` twice to move the selected session to the system trash. Raid will not delete the current session or a session locked by another Raid process.

## Sessions

Raid stores every session in its own SQLite database under:

```text
~/.raid/agent/sessions/<project>/<title>--<session-id>.db
```

The project directory comes from the canonical workspace path, so sessions from different projects stay separate. A new session starts as `New session`. Raid sends only the first user message to the selected text-generation model and renames the database when the generated title arrives. This title request runs separately from the main response.

The resume picker scans the current project's databases each time it opens, so a newly created or renamed session appears without restarting Raid.

## Context compaction

Raid tracks context usage against the selected model's limit. Before the next message would consume the reserved space, Raid uses the current chat model to summarize older context and keeps the recent conversation intact.

Run `/compact` at any time to compact manually. You can add a focus when a detail must survive the summary:

```text
/compact preserve the database migration decisions
```

Compaction records are stored in the session database, so a resumed session continues from the same compacted context.

## Local data

Raid writes its data to `~/.raid/agent` by default:

```text
auth.json          saved provider API keys
settings.json      selected chat and text-generation models
catalog-*.json     cached model catalogs
sessions/          project directories and session databases
```

On Unix, Raid creates private directories with mode `0700` and private files with mode `0600`. Set `RAID_AGENT_DIR` to move all Raid data somewhere else:

```bash
export RAID_AGENT_DIR=/path/to/raid-data
```

## Development

```bash
cargo fmt --all --check
cargo test --workspace
cargo build --release
```
