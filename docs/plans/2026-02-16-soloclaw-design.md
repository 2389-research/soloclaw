# soloclaw Design

A full-screen ratatui TUI agent powered by mux-rs, with an openclaw-style layered tool approval system.

## Goals

- General-purpose conversational agent with configurable LLM provider
- Built-in tools (bash, file ops, search, web) plus MCP server tools
- Streaming responses (tokens appear as they arrive)
- Layered approval system: security levels, ask modes, persistent allowlists, command analysis

## Architecture

### Project Structure

```
soloclaw/
├── Cargo.toml              # standalone, path dep on ../mux-rs
├── src/
│   ├── main.rs             # entry point, CLI parsing, bootstrap
│   ├── app.rs              # App struct, event loop orchestrator
│   ├── config.rs           # load ~/.soloclaw/ config and .mcp.json
│   ├── approval/
│   │   ├── mod.rs
│   │   ├── types.rs        # SecurityLevel, AskMode, AskFallback enums
│   │   ├── policy.rs       # decision logic (requires_approval equivalent)
│   │   ├── allowlist.rs    # persistent allowlist: load/save/match
│   │   ├── analysis.rs     # shell parsing, safe-bin detection
│   │   └── engine.rs       # ApprovalEngine: check → ask → resolve
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── loop.rs         # stream responses, dispatch tool calls
│   │   └── provider.rs     # LLM provider selection
│   └── tui/
│       ├── mod.rs
│       ├── ui.rs           # ratatui layout and rendering
│       ├── state.rs        # messages, scroll, input buffer
│       ├── input.rs        # keyboard handling
│       └── widgets/
│           ├── chat.rs     # message rendering
│           ├── status.rs   # status bar
│           └── approval.rs # inline approval prompt
```

**Dependency flow:** `main` → `App` → `Agent Loop` ↔ `Approval Engine` ↔ `TUI`

### Communication

The TUI runs on the main thread. The agent loop runs in a tokio task. They communicate via channels:

- `AgentEvent` (agent → TUI, `tokio::mpsc`): streaming text, tool calls, tool results, approval requests
- `UserEvent` (TUI → agent, `tokio::mpsc`): user messages
- Approval resolution: `tokio::oneshot` per request — agent blocks, TUI sends decision

## Approval Engine

The approval system mirrors openclaw's three-layer design.

### Types

```rust
enum SecurityLevel { Deny, Allowlist, Full }
enum AskMode { Off, OnMiss, Always }
enum AskFallback { Deny, Allowlist, Full }
enum ApprovalDecision { AllowOnce, AllowAlways, Deny }
```

### Decision Flow

1. `SecurityLevel::Deny` → block unconditionally
2. `SecurityLevel::Allowlist` + tool matches allowlist → allow silently
3. `SecurityLevel::Allowlist` + no match + `AskMode::OnMiss` → prompt user
4. `SecurityLevel::Full` + `AskMode::Always` → prompt user
5. `SecurityLevel::Full` + `AskMode::Off` → allow silently

### Command Analysis (bash tool only)

- Parse shell pipelines into segments
- Resolve executables against PATH
- Safe-bin list (grep, jq, sort, cut, etc.) auto-approve on stdin-only input
- Other tools skip analysis, go straight to security/ask evaluation

### Persistent Allowlist (`~/.soloclaw/approvals.json`)

```json
{
  "version": 1,
  "defaults": {
    "security": "allowlist",
    "ask": "on-miss",
    "ask_fallback": "deny"
  },
  "tools": {
    "bash": {
      "security": "allowlist",
      "ask": "on-miss",
      "allowlist": [
        { "pattern": "/usr/bin/ls", "added_at": "2026-02-16T..." }
      ]
    },
    "read_file": {
      "security": "full",
      "ask": "off"
    },
    "*": {
      "security": "allowlist",
      "ask": "on-miss"
    }
  }
}
```

"Allow Always" adds the tool name (or resolved executable pattern for bash) to the allowlist and persists. Approval requests timeout after 2 minutes; `ask_fallback` determines the outcome.

## TUI

### Layout

```
┌──────────────────────────────────────────────────┐
│  soloclaw · claude-sonnet-4 · 12 tools         │  header bar
├──────────────────────────────────────────────────┤
│                                                  │
│  You: What files are in this directory?           │
│                                                  │
│  🔧 bash("ls -la")                    [allowed]  │  tool call
│  ┃ total 24                                      │
│  ┃ drwxr-xr-x  5 harper staff 160 Feb 16 ...    │
│                                                  │
│  Assistant: Here are the files...                 │
│                                                  │
│  🔧 bash("rm -rf /tmp/data")  ⏳ APPROVE?       │  approval prompt
│  ┃  [Allow Once]  [Always Allow]  [Deny]         │
│                                                  │
├──────────────────────────────────────────────────┤
│ > type your message here...                      │  input area
├──────────────────────────────────────────────────┤
│ tokens: 1.2k/4k │ tools: 12 │ ⚡ streaming      │  status bar
└──────────────────────────────────────────────────┘
```

### Behaviors

- Chat scrolls with user messages, streaming assistant text, tool calls, and results inline
- Tool calls display name + params summary with status: `[allowed]`, `[denied]`, or approval prompt
- Approval prompt: three options via arrow keys or `1`/`2`/`3` hotkeys, blocks input until resolved or timeout
- Input: multi-line (Shift+Enter), Enter to send
- Status bar: token usage, tool count, streaming indicator, model name

## Agent Loop

Uses mux-rs `create_message_stream()` for streaming. On each response:

1. Forward text deltas to TUI as `AgentEvent::TextDelta`
2. On tool_use blocks: send through ApprovalEngine
3. If approved: execute tool, send result to TUI, add to conversation history
4. If denied: add denial result to history, continue
5. If response has tool calls: loop back to LLM with results
6. If no tool calls: break inner loop, wait for next user message

## Configuration

### `~/.soloclaw/config.toml`

```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
max_tokens = 4096

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "allowlist"
ask = "on-miss"
ask_fallback = "deny"
timeout_seconds = 120
```

### CLI Flags (override config)

- `--provider <name>` — LLM provider
- `--model <name>` — model
- `--security <level>` — default security level

### MCP Config

Reads `.mcp.json` from current directory or `~/.mcp.json` (same format as mux-rs).

## Error Handling

- LLM errors: display as system messages in chat, don't crash
- MCP failures: warn on startup, continue without that server
- Tool errors: display inline with the tool call
- Approval timeouts: resolve per `ask_fallback`, show status in chat
- TUI panics: restore terminal via `std::panic::set_hook` before printing

## Testing

- **Unit:** Approval decision logic, allowlist matching, command analysis parsing
- **Integration:** Agent loop with mock LLM returning tool_use → verify approval flow. Allowlist persistence round-trip
- **E2E:** Full TUI via ratatui `TestBackend`, simulate keystrokes, verify rendered output

## Dependencies

```toml
[dependencies]
mux = { path = "../mux-rs" }
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
dirs = "5"
anyhow = "1"
dotenvy = "0.15"
glob = "0.3"
```
