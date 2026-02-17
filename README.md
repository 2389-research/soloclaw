# soloclaw

A terminal AI agent with layered tool approval — built in Rust.

```
┌──────────────────────────────────────────────────┐
│  soloclaw · claude-sonnet-4-5 · 12 tools         │
├──────────────────────────────────────────────────┤
│                                                  │
│ ❯ What files are in this directory?              │
│                                                  │
│ ⚙ bash({"command":"ls -la"})          [allowed]  │
│    total 24                                      │
│    -rw-r--r--  1 user  staff  643 pyproject.toml │
│                                                  │
│ ⏺ Here are the files in your directory...        │
│                                                  │
├──────────────────────────────────────────────────┤
│ APPROVE?  bash({"command":"rm -rf /tmp/old"})    │
│  [1] Allow Once   [2] Always Allow   [3] Deny   │
├──────────────────────────────────────────────────┤
│ > _                                              │
├──────────────────────────────────────────────────┤
│ tokens: 1.2k                                     │
└──────────────────────────────────────────────────┘
```

## Features

- **Multi-provider LLM support** — Anthropic, OpenAI, Gemini, OpenRouter, Ollama
- **Streaming TUI** — full-screen ratatui interface with real-time token streaming
- **5 built-in tools** — bash, read_file, write_file, list_files, search
- **MCP extension** — connect additional tools via Model Context Protocol servers
- **Layered approval engine** — deny, allowlist, or full-trust security levels with persistent allow rules
- **Context files** — drop `SOUL.md`, `AGENTS.md`, or `TOOLS.md` in your project to shape agent behavior
- **Skill injection** — load `SKILL.md` instructions from multiple directories into the system prompt
- **XDG-compliant config** — config, secrets, and approvals stored under `$XDG_CONFIG_HOME/soloclaw/`
- **Shell safety analysis** — auto-approves safe read-only commands (grep, cat, ls, etc.)
- **Unicode-safe editing** — full UTF-8 input with correct cursor positioning

## Quickstart

```bash
# 1. Build and install (requires mux-rs sibling — see Building section)
cargo install --path .

# 2. Run interactive setup (writes config, API keys, approvals)
soloclaw setup

# 3. Launch
soloclaw
```

## Usage

### CLI

```
soloclaw                             # launch with config defaults
soloclaw setup                        # interactive first-run setup
soloclaw --provider openai            # override provider
soloclaw --model claude-opus-4-6      # override model
soloclaw --security full              # trust all tools (no approval prompts)
```

Flags override values from `config.toml` for that session.

### Keyboard Shortcuts

| Key | Action |
|---|---|
| `Enter` | Send message |
| `Ctrl+C` / `Esc` | Quit |
| `←` / `→` | Move cursor in input |
| `↑` / `↓` / `PgUp` / `PgDn` | Scroll chat history |
| `Home` / `End` | Jump to start/end of input |
| `Backspace` / `Delete` | Delete characters |
| `1` / `2` / `3` | Quick-select approval option |
| `←` / `→` (during approval) | Navigate approval choices |
| Mouse scroll | Scroll chat |

## Configuration

All config lives under `$XDG_CONFIG_HOME/soloclaw/` (typically `~/.config/soloclaw/`). Run `soloclaw setup` to generate defaults.

### config.toml

```toml
[llm]
provider = "anthropic"                # anthropic, openai, gemini, openrouter, ollama
model = "claude-sonnet-4-5-20250929"
max_tokens = 4096

[llm.anthropic]
base_url = "https://api.anthropic.com"

[llm.openai]
base_url = "https://api.openai.com/v1"

[llm.gemini]
base_url = "https://generativelanguage.googleapis.com/v1beta"

[llm.openrouter]
base_url = "https://openrouter.ai/api/v1"

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "allowlist"    # deny | allowlist | full
ask = "on-miss"           # off | on-miss | always
ask_fallback = "deny"     # deny | allowlist | full
timeout_seconds = 120

[permissions]
bypass_approvals = false

[skills]
enabled = true
include_xdg_config = true
include_workspace = true
include_agents_home = true
include_codex_home = true
max_files = 24
max_file_bytes = 131072       # 128 KB per skill file
max_total_chars = 32000       # total budget across all skills
```

### secrets.env

API keys are stored in `secrets.env` (chmod 600). The `soloclaw setup` wizard prompts for these:

```
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
GEMINI_API_KEY=...
OPENROUTER_API_KEY=...
```

Keys are also loaded from `.env` in the working directory and from the shell environment.

### .mcp.json

MCP server definitions follow the same format as Claude Desktop. Place in the working directory or `~/`:

```json
{
  "mcpServers": {
    "my-server": {
      "command": "npx",
      "args": ["-y", "my-mcp-server"],
      "env": { "API_KEY": "..." }
    }
  }
}
```

Tools from connected MCP servers appear alongside built-in tools and go through the same approval engine.

## Context Files

Drop these files in your project root to customize agent behavior. Soloclaw loads them automatically at startup.

| File | Purpose |
|---|---|
| `.soloclaw.md` | Project-specific instructions — equivalent to a project config for the agent |
| `SOUL.md` | Agent persona and tone — if present, the agent embodies this personality |
| `AGENTS.md` | Behavioral guidelines — how the agent should approach tasks |
| `TOOLS.md` | Tool usage notes — guidance for how to use external tools (does **not** control tool availability) |

All files are optional. Only non-empty files are loaded. Contents are injected into the system prompt under a `## Project Context` section.

## Skill System

Skills are `SKILL.md` files that inject task-specific instructions into the system prompt. Soloclaw searches four roots (all configurable):

1. **XDG config** — `$XDG_CONFIG_HOME/soloclaw/skills/<name>/SKILL.md`
2. **Workspace** — `./skills/<name>/SKILL.md`
3. **Agents home** — `~/.agents/skills/<name>/SKILL.md`
4. **Codex home** — `$CODEX_HOME/skills/<name>/SKILL.md` or `~/.codex/skills/<name>/SKILL.md`

```
skills/
  my-skill/
    SKILL.md       # loaded into the system prompt
  another-skill/
    SKILL.md
```

Limits prevent prompt bloat: 24 files max, 128 KB per file, 32,000 characters total.

## Approval Engine

The approval engine controls what tools the agent can execute. It combines three layers:

### Security Levels

| Level | Behavior |
|---|---|
| `deny` | Block all tool calls |
| `allowlist` | Allow only tools/commands matching `approvals.json` patterns |
| `full` | Trust all tool calls (still subject to ask mode) |

### Ask Modes

| Mode | Behavior |
|---|---|
| `off` | Never prompt — use security level rules directly |
| `on-miss` | Prompt only when a tool call doesn't match the allowlist |
| `always` | Prompt for every tool call |

### Persistent Allowlist

When you choose **Always Allow** for a tool call, soloclaw records the pattern in `approvals.json`. Future matching calls are auto-approved.

For bash commands, the engine resolves the executable to its absolute path (e.g., `/usr/bin/grep`) and stores that as the pattern. It also maintains a built-in safe list of read-only binaries (awk, cat, grep, head, jq, ls, sort, wc, etc.) that are auto-approved when they read from stdin only.

### Approval Flow

```
Tool call arrives
  → Security level check (deny blocks immediately)
  → Ask mode check (always → prompt)
  → Allowlist lookup (match → allow, miss + on-miss → prompt)
  → User decides: Allow Once / Always Allow / Deny
  → Always Allow → persist to approvals.json
```

Approval prompts have a configurable timeout (default 120 seconds). Timeout = deny.

## Supported Providers

| Provider | Env Var | Default Model |
|---|---|---|
| `anthropic` | `ANTHROPIC_API_KEY` | `claude-sonnet-4-5-20250929` |
| `openai` | `OPENAI_API_KEY` | `gpt-5.2` |
| `gemini` | `GEMINI_API_KEY` | `gemini-2.5-pro` |
| `openrouter` | `OPENROUTER_API_KEY` | `anthropic/claude-sonnet-4` |
| `ollama` | — | `llama3.2` |

All providers support custom `base_url` in config for proxies or self-hosted endpoints.

## Architecture

```
src/
  main.rs              # CLI entry point (clap)
  lib.rs               # crate root — re-exports all modules
  app.rs               # orchestrator: wires TUI, agent loop, tools, approval
  config.rs            # XDG config loading, setup wizard, MCP config
  prompt.rs            # dynamic system prompt builder, context/skill loading
  agent/
    mod.rs             # module root
    provider.rs        # LLM client factory (anthropic, openai, gemini, etc.)
    loop.rs            # streaming agent loop: conversation turns, tool dispatch
  approval/
    mod.rs             # module root
    policy.rs          # pure decision logic (security × ask × allowlist)
    analysis.rs        # shell command parsing and safe-binary detection
    allowlist.rs       # persistent JSON allowlist (approvals.json)
    engine.rs          # orchestrator: check → resolve → persist
  tui/
    mod.rs             # module root
    state.rs           # TUI state: messages, input buffer, approval prompt
    ui.rs              # ratatui rendering (layout, styling, content)
    input.rs           # keyboard/input event handling
    widgets/
      chat.rs          # chat message rendering with line wrapping
      approval.rs      # inline approval prompt widget
```

The `mux` crate (`../mux-rs`) provides the LLM client abstraction, tool registry, MCP client, and message types.

## Building & Testing

Soloclaw depends on `mux-rs` as a sibling path dependency. Clone both repositories:

```bash
git clone <repo-url> soloclaw
git clone <mux-repo-url> mux-rs    # must be at ../mux-rs relative to soloclaw

cd soloclaw
cargo build
cargo test
```

The test suite covers:
- Unit tests for prompt assembly, config parsing, approval policy, input handling, TUI state
- Integration tests for the approval engine, system prompt builder, and TUI rendering

## License

TBD
