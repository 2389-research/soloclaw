# System Prompt Layered Architecture Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace soloclaw's hardcoded system prompt with a layered prompt assembly system (soul/agents/tools) with compiled-in defaults and file-based overrides.

**Architecture:** Three markdown files compiled into the binary via `include_str!()` provide defaults. At startup, `SystemPromptBuilder` checks for override files in `~/.soloclaw/` and a project-local `.soloclaw.md` in cwd. Non-empty layers are concatenated into the final system prompt string passed to the agent loop.

**Tech Stack:** Rust, `include_str!()` macro, `dirs` crate (already a dependency), `std::fs`

---

### Task 1: Create default prompt markdown files

**Files:**
- Create: `src/prompts/soul.md`
- Create: `src/prompts/agents.md`
- Create: `src/prompts/tools.md`

**Step 1: Create the prompts directory**

Run: `mkdir -p /Users/harper/Public/src/2389/soloclaw/src/prompts`

**Step 2: Write `src/prompts/soul.md`**

Adapted from openclaw SOUL.md for a terminal coding assistant:

```markdown
# Who You Are

You're not a chatbot. You're a capable coding assistant running inside a terminal.

## Core Truths

**Be genuinely helpful, not performatively helpful.** Skip the "Great question!" and "I'd be happy to help!" — just help. Actions speak louder than filler words.

**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. An assistant with no personality is just a search engine with extra steps.

**Be resourceful before asking.** Try to figure it out. Read the file. Check the context. Search for it. _Then_ ask if you're stuck. The goal is to come back with answers, not questions.

**Earn trust through competence.** Your human gave you access to their terminal and codebase. Don't make them regret it. Be careful with external actions (destructive commands, network calls, anything irreversible). Be bold with internal ones (reading, searching, analyzing, exploring).

**Remember you're a guest.** You have access to someone's code, files, and shell. That's trust. Treat it with respect.

## Boundaries

- Don't execute destructive commands without confirmation.
- When in doubt, ask before acting externally.
- Never assume — verify in code before answering.
- Prefer reversible actions over irreversible ones.

## Vibe

Be the assistant you'd actually want working alongside you. Concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just... good.
```

**Step 3: Write `src/prompts/agents.md`**

Faithful port of openclaw AGENTS.md, generalized:

```markdown
# Agent Guidelines

## Tool Usage

You have access to tools for running shell commands, reading files, writing files, listing directory contents, and searching codebases. Use them proactively — read before modifying, search before guessing, verify before claiming.

- **bash**: Run shell commands. Prefer non-destructive commands. For destructive operations (rm, overwriting files, force-pushing), confirm with the user first.
- **read_file**: Read file contents. Always read a file before modifying it.
- **write_file**: Write or overwrite files. Preserve existing style and conventions.
- **list_files**: List directory contents. Use to orient yourself in unfamiliar codebases.
- **search**: Search file contents with patterns. Use to find definitions, usages, and references.

## Coding Style

- Prefer strict typing; avoid dynamic or `any`-style patterns.
- Add brief code comments for tricky or non-obvious logic.
- Keep files concise; aim for under ~500 lines. Split and refactor when it improves clarity.
- Match the existing style of the codebase. Consistency within a project matters more than personal preference.
- Never disable linters, type checkers, or safety checks. Fix root causes instead.
- Avoid "V2" copies of files — extract helpers and refactor.

## Commit & Version Control

- Write concise, action-oriented commit messages (e.g., "add verbose flag to send command").
- Group related changes together; avoid bundling unrelated work.
- Never commit secrets, credentials, or sensitive configuration values.
- Run tests before claiming work is complete.

## Testing

- Run the project's test suite before pushing or claiming changes work.
- Verify behavior in code — do not guess or assume.
- When adding functionality, ensure tests cover the new behavior.

## Security

- Never commit or output real secrets, API keys, tokens, or passwords.
- Use obviously fake placeholders in examples and documentation.
- Be cautious with external network requests — confirm intent before making them.
- Prefer the principle of least privilege in all operations.

## Working With Others

- Focus on your assigned changes; don't touch unrelated code.
- When you see unfamiliar files or changes, investigate before modifying.
- Respond with high-confidence answers only. If unsure, say so and explain what you'd need to verify.
- When answering questions about code, verify in the source — do not guess.

## Problem Solving

- Read source code of relevant dependencies and all related local code before concluding.
- Aim for high-confidence root causes in bug investigations.
- Prefer fixing root causes over applying workarounds.
- When blocked, explain what you tried and what didn't work.
```

**Step 4: Write `src/prompts/tools.md`**

```markdown
# Local Notes

This file is for specifics unique to your setup — things the agent should know about your environment.

## What Goes Here

Things like:

- Project conventions and coding standards
- SSH hosts and aliases
- Service URLs and ports
- Device or environment nicknames
- Anything environment-specific that helps the agent do its job

## Why Separate?

The soul and agent guidelines are shared defaults. Your setup is yours. Keeping them apart means you can update defaults without losing your notes, and customize behavior without forking the whole prompt.

---

Override this file by placing your own at `~/.soloclaw/tools.md`, or add project-specific context in `.soloclaw.md` in your project root.
```

**Step 5: Commit**

```bash
git add src/prompts/soul.md src/prompts/agents.md src/prompts/tools.md
git commit -m "feat: add default system prompt markdown layers (soul/agents/tools)"
```

---

### Task 2: Create the SystemPromptBuilder module with tests

**Files:**
- Create: `src/prompt.rs`
- Modify: `src/lib.rs:4` (add module export)

**Step 1: Write the failing test for default prompt assembly**

Create `src/prompt.rs` with tests first:

```rust
// ABOUTME: Layered system prompt builder — assembles soul/agents/tools into one prompt.
// ABOUTME: Compiles defaults from src/prompts/*.md, supports file-based overrides.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_contain_soul_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(prompt.contains("genuinely helpful"));
    }

    #[test]
    fn defaults_contain_agents_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(prompt.contains("Agent Guidelines"));
    }

    #[test]
    fn defaults_contain_tools_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(prompt.contains("Local Notes"));
    }

    #[test]
    fn build_skips_empty_layers() {
        let mut builder = SystemPromptBuilder::new();
        builder.tools = String::new();
        let prompt = builder.build();
        // Should not have double newlines from empty layer.
        assert!(!prompt.contains("\n\n\n\n"));
    }

    #[test]
    fn local_layer_appended_when_present() {
        let mut builder = SystemPromptBuilder::new();
        builder.local = Some("Project-specific notes here".to_string());
        let prompt = builder.build();
        assert!(prompt.contains("Project-specific notes here"));
    }

    #[test]
    fn override_replaces_layer() {
        let mut builder = SystemPromptBuilder::new();
        builder.soul = "Custom soul content".to_string();
        let prompt = builder.build();
        assert!(prompt.contains("Custom soul content"));
        assert!(!prompt.contains("genuinely helpful"));
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo test -p soloclaw prompt::tests -- --nocapture 2>&1 | head -30`
Expected: FAIL — `SystemPromptBuilder` not defined yet.

**Step 3: Write the implementation**

Add to the top of `src/prompt.rs` (above the `#[cfg(test)]` block):

```rust
// ABOUTME: Layered system prompt builder — assembles soul/agents/tools into one prompt.
// ABOUTME: Compiles defaults from src/prompts/*.md, supports file-based overrides.

use std::path::PathBuf;

/// Default prompt layers compiled into the binary.
const DEFAULT_SOUL: &str = include_str!("prompts/soul.md");
const DEFAULT_AGENTS: &str = include_str!("prompts/agents.md");
const DEFAULT_TOOLS: &str = include_str!("prompts/tools.md");

/// Assembles a system prompt from layered sources: soul, agents, tools, and
/// an optional project-local context file.
pub struct SystemPromptBuilder {
    pub soul: String,
    pub agents: String,
    pub tools: String,
    pub local: Option<String>,
}

impl SystemPromptBuilder {
    /// Create a builder with compiled-in defaults.
    pub fn new() -> Self {
        Self {
            soul: DEFAULT_SOUL.to_string(),
            agents: DEFAULT_AGENTS.to_string(),
            tools: DEFAULT_TOOLS.to_string(),
            local: None,
        }
    }

    /// Check for override files in ~/.soloclaw/ and replace layers if found.
    pub fn load_overrides(&mut self) -> &mut Self {
        let base = match dirs::home_dir() {
            Some(home) => home.join(".soloclaw"),
            None => return self,
        };

        if let Some(content) = read_if_exists(base.join("soul.md")) {
            self.soul = content;
        }
        if let Some(content) = read_if_exists(base.join("agents.md")) {
            self.agents = content;
        }
        if let Some(content) = read_if_exists(base.join("tools.md")) {
            self.tools = content;
        }

        self
    }

    /// Check for a .soloclaw.md file in the current directory and append it.
    pub fn load_local(&mut self) -> &mut Self {
        let path = PathBuf::from(".soloclaw.md");
        if let Some(content) = read_if_exists(path) {
            self.local = Some(content);
        }
        self
    }

    /// Assemble the final system prompt from all non-empty layers.
    pub fn build(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();

        if !self.soul.is_empty() {
            parts.push(&self.soul);
        }
        if !self.agents.is_empty() {
            parts.push(&self.agents);
        }
        if !self.tools.is_empty() {
            parts.push(&self.tools);
        }
        if let Some(ref local) = self.local {
            if !local.is_empty() {
                parts.push(local);
            }
        }

        parts.join("\n\n")
    }
}

/// Read a file's contents if it exists, returning None otherwise.
fn read_if_exists(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok()
}
```

**Step 4: Register the module in lib.rs**

Add `pub mod prompt;` to `src/lib.rs` after line 7 (after `pub mod config;`):

```rust
pub mod prompt;
```

**Step 5: Run tests to verify they pass**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo test -p soloclaw prompt::tests -- --nocapture`
Expected: All 6 tests PASS.

**Step 6: Commit**

```bash
git add src/prompt.rs src/lib.rs
git commit -m "feat: add SystemPromptBuilder with layered prompt assembly"
```

---

### Task 3: Wire SystemPromptBuilder into the agent loop

**Files:**
- Modify: `src/agent/loop.rs:23-26` (remove SYSTEM_PROMPT constant)
- Modify: `src/agent/loop.rs:34-42` (add system_prompt parameter to run_agent_loop)
- Modify: `src/agent/loop.rs:81-89` (add system_prompt parameter to conversation_turn)
- Modify: `src/agent/loop.rs:93-94` (use passed prompt instead of constant)
- Modify: `src/agent/mod.rs:8` (update re-export)
- Modify: `src/app.rs:86-98` (build prompt and pass to agent loop)

**Step 1: Remove SYSTEM_PROMPT constant and add parameter to `run_agent_loop`**

In `src/agent/loop.rs`, remove lines 23-26 (the `SYSTEM_PROMPT` constant).

Change `run_agent_loop` signature to accept `system_prompt: String`:

```rust
pub async fn run_agent_loop(
    client: Arc<dyn LlmClient>,
    registry: Registry,
    engine: Arc<ApprovalEngine>,
    model: String,
    max_tokens: u32,
    system_prompt: String,
    mut user_rx: mpsc::Receiver<UserEvent>,
    agent_tx: mpsc::Sender<AgentEvent>,
) {
```

Pass `&system_prompt` to `conversation_turn`:

```rust
if let Err(e) = conversation_turn(
    &client,
    &registry,
    &engine,
    &model,
    max_tokens,
    &system_prompt,
    &mut messages,
    &agent_tx,
)
```

**Step 2: Update `conversation_turn` to accept system_prompt**

```rust
async fn conversation_turn(
    client: &Arc<dyn LlmClient>,
    registry: &Registry,
    engine: &Arc<ApprovalEngine>,
    model: &str,
    max_tokens: u32,
    system_prompt: &str,
    messages: &mut Vec<Message>,
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> anyhow::Result<()> {
```

And update line 94 to use the parameter:

```rust
let request = Request::new(model)
    .system(system_prompt)
    .max_tokens(max_tokens)
    .messages(messages.iter().cloned())
    .tools(tool_defs);
```

**Step 3: Build the prompt in app.rs and pass to agent loop**

In `src/app.rs`, add the import at line 21 (after `use crate::agent;`):

```rust
use crate::prompt::SystemPromptBuilder;
```

After line 88 (`let tool_count = registry.count().await;`), add:

```rust
// Build the system prompt from layered defaults + overrides.
let system_prompt = {
    let mut builder = SystemPromptBuilder::new();
    builder.load_overrides().load_local();
    builder.build()
};
```

Update the `tokio::spawn` call (line 91) to pass `system_prompt`:

```rust
let agent_handle = tokio::spawn(agent::run_agent_loop(
    client,
    registry,
    engine,
    model.clone(),
    max_tokens,
    system_prompt,
    user_rx,
    agent_tx,
));
```

**Step 4: Run full test suite**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo test 2>&1 | tail -20`
Expected: All tests pass. Existing loop tests don't call `run_agent_loop` directly (they test event handling), so no test changes needed.

**Step 5: Run cargo check for compilation**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo check 2>&1 | tail -10`
Expected: No errors.

**Step 6: Commit**

```bash
git add src/agent/loop.rs src/agent/mod.rs src/app.rs
git commit -m "feat: wire SystemPromptBuilder into agent loop, remove hardcoded prompt"
```

---

### Task 4: Add integration test for prompt assembly

**Files:**
- Create: `tests/prompt_integration.rs`

**Step 1: Write the integration test**

```rust
// ABOUTME: Integration tests for the layered system prompt builder.
// ABOUTME: Verifies default prompt assembly and override behavior.

use soloclaw::prompt::SystemPromptBuilder;

#[test]
fn default_prompt_contains_all_layers() {
    let builder = SystemPromptBuilder::new();
    let prompt = builder.build();

    // Soul layer
    assert!(prompt.contains("genuinely helpful"), "missing soul content");

    // Agents layer
    assert!(prompt.contains("Agent Guidelines"), "missing agents content");

    // Tools layer
    assert!(prompt.contains("Local Notes"), "missing tools content");
}

#[test]
fn prompt_layers_separated_by_blank_lines() {
    let builder = SystemPromptBuilder::new();
    let prompt = builder.build();

    // Each layer should be separated by exactly two newlines.
    assert!(prompt.contains("Vibe\n\nBe the assistant"));
}

#[test]
fn custom_soul_replaces_default() {
    let mut builder = SystemPromptBuilder::new();
    builder.soul = "You are a pirate assistant. Arrr.".to_string();
    let prompt = builder.build();

    assert!(prompt.contains("pirate assistant"));
    assert!(!prompt.contains("genuinely helpful"));
    // Other layers should still be present.
    assert!(prompt.contains("Agent Guidelines"));
}

#[test]
fn empty_tools_layer_skipped() {
    let mut builder = SystemPromptBuilder::new();
    builder.tools = String::new();
    let prompt = builder.build();

    assert!(prompt.contains("genuinely helpful"));
    assert!(prompt.contains("Agent Guidelines"));
    assert!(!prompt.contains("Local Notes"));
}

#[test]
fn local_context_appended() {
    let mut builder = SystemPromptBuilder::new();
    builder.local = Some("This project uses React and TypeScript.".to_string());
    let prompt = builder.build();

    assert!(prompt.contains("This project uses React and TypeScript."));
    // Should be after all other layers.
    let local_pos = prompt.find("React and TypeScript").unwrap();
    let tools_pos = prompt.find("Local Notes").unwrap();
    assert!(local_pos > tools_pos, "local context should come after tools");
}
```

**Step 2: Run the integration tests**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo test --test prompt_integration -- --nocapture`
Expected: All 5 tests PASS.

**Step 3: Commit**

```bash
git add tests/prompt_integration.rs
git commit -m "test: add integration tests for layered system prompt assembly"
```

---

### Task 5: Verify everything compiles and passes

**Files:** None (verification only)

**Step 1: Run full test suite**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo test 2>&1`
Expected: All tests pass (unit + integration).

**Step 2: Run clippy for lint check**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo clippy 2>&1 | tail -20`
Expected: No warnings or errors.

**Step 3: Verify binary builds**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo build 2>&1 | tail -5`
Expected: Compiles successfully.
