# System Prompt Layered Architecture Design

**Date**: 2026-02-16
**Status**: Approved

## Summary

Replace soloclaw's hardcoded 3-line system prompt with a layered prompt assembly
system inspired by openclaw's SOUL.md / AGENTS.md / TOOLS.md architecture.

## Architecture

Three compiled-in default layers stored as `.md` files in `src/prompts/`, embedded
into the binary via `include_str!()`:

```
src/prompts/
├── soul.md      # Personality/philosophy (adapted from openclaw SOUL.md)
├── agents.md    # Technical guidelines (faithful port, generalized)
└── tools.md     # Default tools template (explains what goes here)
```

At runtime, the prompt builder checks for overrides:

1. `~/.soloclaw/soul.md` → overrides the soul layer
2. `~/.soloclaw/agents.md` → overrides the agents layer
3. `~/.soloclaw/tools.md` → overrides the global tools layer
4. `.soloclaw.md` in cwd → appended as a project-local context layer

Final system prompt = `soul + "\n\n" + agents + "\n\n" + tools + "\n\n" + local`
(skipping empty layers).

## Content Layers

### soul.md

Adapted from openclaw's SOUL.md. Core personality for a terminal coding assistant:

- Be genuinely helpful, not performatively helpful. No filler — just help.
- Have opinions. Disagree when appropriate.
- Be resourceful before asking. Read the file, check context, search. Then ask.
- Earn trust through competence. Careful with external actions, bold with internal.
- Remember you're a guest. Respect access to codebase and terminal.
- Boundaries: No destructive commands without confirmation. Ask before acting externally.
- Vibe: Concise when needed, thorough when it matters.

### agents.md

Faithful port of openclaw's AGENTS.md, generalized for any codebase:

- Tool usage: Available tools (bash, file read/write, search) and safe patterns
- Coding style: Strict typing, comments for tricky logic, concise files
- Commit/git guidelines: Concise action-oriented messages, scoped changes
- Testing guidelines: Run tests before claiming done, verify in code
- Security: Never commit secrets, careful with destructive operations
- Multi-agent safety: Don't modify unrelated state, scope to your task
- Agent-specific: Verify answers in code, high-confidence answers only

### tools.md

Template explaining what goes here — environment-specific notes like camera names,
SSH hosts, device nicknames. Essentially openclaw's TOOLS.md template.

## Implementation

### New files

- `src/prompts/soul.md` — compiled-in default soul
- `src/prompts/agents.md` — compiled-in default guidelines
- `src/prompts/tools.md` — compiled-in default tools template
- `src/prompt.rs` — SystemPromptBuilder module

### Changes to existing files

- `src/lib.rs` — add `pub mod prompt;`
- `src/agent/loop.rs` — remove `SYSTEM_PROMPT` constant, accept assembled prompt
  as parameter to `run_agent_loop()`, pass through to `conversation_turn()`
- `src/app.rs` — call `SystemPromptBuilder::new().build()` at startup, pass into
  agent loop

### SystemPromptBuilder API

```rust
pub struct SystemPromptBuilder {
    soul: String,
    agents: String,
    tools: String,
    local: Option<String>,
}

impl SystemPromptBuilder {
    pub fn new() -> Self { /* loads defaults via include_str! */ }
    pub fn load_overrides(&mut self) -> &mut Self { /* checks ~/.soloclaw/*.md */ }
    pub fn load_local(&mut self) -> &mut Self { /* checks .soloclaw.md in cwd */ }
    pub fn build(&self) -> String { /* concatenates non-empty layers */ }
}
```

### Testing

- Unit tests: defaults load, override replaces layer, local appends, empty layers skipped
- Integration test: assembled prompt contains expected sections
- Existing tests untouched (agent loop signature changes but test mocks pass any string)

### No changes to

- config.toml schema
- Approval engine
- TUI rendering
- MCP loading
