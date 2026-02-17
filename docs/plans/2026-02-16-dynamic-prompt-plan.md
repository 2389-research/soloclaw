# Dynamic System Prompt Builder Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the layered SystemPromptBuilder with a dynamic prompt builder that faithfully ports openclaw's `buildAgentSystemPrompt()` — assembling the system prompt at runtime from capabilities, environment, and workspace context files.

**Architecture:** Single `build_system_prompt()` function takes a params struct with runtime info (tools, workspace, OS, model, context files) and returns an assembled prompt string with conditional sections matching openclaw's structure. Context files are loaded from the workspace at startup.

**Tech Stack:** Rust, std::env, chrono (for timezone), dirs crate

---

### Task 1: Delete old layered prompt system

**Files:**
- Delete: `src/prompts/soul.md`
- Delete: `src/prompts/agents.md`
- Delete: `src/prompts/tools.md`
- Delete: `src/prompts/` directory
- Rewrite: `src/prompt.rs` (gut it, keep just the ABOUTME + module structure)

**Step 1: Remove the prompts directory and its contents**

```bash
rm -rf src/prompts/
```

**Step 2: Replace src/prompt.rs with a stub**

Replace the entire file with:

```rust
// ABOUTME: Dynamic system prompt builder — assembles prompt from runtime capabilities.
// ABOUTME: Faithful port of openclaw's buildAgentSystemPrompt() pattern.

use std::collections::HashMap;

/// A context file loaded from the workspace to inject into the system prompt.
#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}

/// Parameters for building the system prompt at runtime.
#[derive(Debug, Clone)]
pub struct SystemPromptParams {
    /// Tool names available in the registry.
    pub tool_names: Vec<String>,
    /// Tool name -> description mapping.
    pub tool_summaries: HashMap<String, String>,
    /// Current working directory.
    pub workspace_dir: String,
    /// Operating system name (e.g., "macos", "linux").
    pub os: String,
    /// CPU architecture (e.g., "aarch64", "x86_64").
    pub arch: String,
    /// User's shell (e.g., "/bin/zsh").
    pub shell: String,
    /// LLM model name.
    pub model: String,
    /// Context files loaded from the workspace.
    pub context_files: Vec<ContextFile>,
}

/// Build the system prompt from runtime parameters.
///
/// Mirrors openclaw's buildAgentSystemPrompt(): assembles sections conditionally
/// based on available capabilities and environment.
pub fn build_system_prompt(params: &SystemPromptParams) -> String {
    todo!("implement in Task 2")
}

/// Load context files from the workspace directory.
///
/// Searches for: .soloclaw.md, SOUL.md, AGENTS.md, TOOLS.md
pub fn load_context_files(workspace_dir: &str) -> Vec<ContextFile> {
    todo!("implement in Task 3")
}
```

**Step 3: Update src/app.rs import**

Change `use crate::prompt::SystemPromptBuilder;` to `use crate::prompt::{build_system_prompt, load_context_files, SystemPromptParams};`

Comment out or stub the prompt building temporarily so it compiles:

```rust
// Temporarily use a placeholder until Task 2 implements build_system_prompt.
let system_prompt = "You are a personal assistant running inside SingleClaw.".to_string();
```

**Step 4: Verify compilation**

Run: `cargo check`
Expected: Compiles (with dead_code warnings for the todo!() functions, which is fine).

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove layered prompt system, stub dynamic builder"
```

---

### Task 2: Implement build_system_prompt()

**Files:**
- Rewrite: `src/prompt.rs` (replace todo!() with full implementation)

**Step 1: Write tests for build_system_prompt**

Add at the bottom of `src/prompt.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn base_params() -> SystemPromptParams {
        SystemPromptParams {
            tool_names: vec!["bash".to_string(), "read_file".to_string()],
            tool_summaries: {
                let mut m = HashMap::new();
                m.insert("bash".to_string(), "Execute a bash command".to_string());
                m.insert("read_file".to_string(), "Read file contents".to_string());
                m
            },
            workspace_dir: "/tmp/test-project".to_string(),
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
            shell: "/bin/zsh".to_string(),
            model: "claude-sonnet-4".to_string(),
            context_files: vec![],
        }
    }

    #[test]
    fn prompt_starts_with_identity() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.starts_with("You are a personal assistant running inside SingleClaw."));
    }

    #[test]
    fn prompt_contains_tooling_section() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Tooling"));
        assert!(prompt.contains("- bash: Execute a bash command"));
        assert!(prompt.contains("- read_file: Read file contents"));
    }

    #[test]
    fn prompt_contains_tool_call_style() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Tool Call Style"));
    }

    #[test]
    fn prompt_contains_safety_section() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Safety"));
        assert!(prompt.contains("self-preservation"));
    }

    #[test]
    fn prompt_contains_workspace() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Workspace"));
        assert!(prompt.contains("/tmp/test-project"));
    }

    #[test]
    fn prompt_contains_date_time() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Current Date & Time"));
    }

    #[test]
    fn prompt_contains_runtime() {
        let prompt = build_system_prompt(&base_params());
        assert!(prompt.contains("## Runtime"));
        assert!(prompt.contains("os=macos (aarch64)"));
        assert!(prompt.contains("model=claude-sonnet-4"));
        assert!(prompt.contains("shell=/bin/zsh"));
    }

    #[test]
    fn prompt_with_context_files() {
        let mut params = base_params();
        params.context_files = vec![
            ContextFile {
                path: "AGENTS.md".to_string(),
                content: "# My Guidelines\nBe helpful.".to_string(),
            },
        ];
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("# Project Context"));
        assert!(prompt.contains("## AGENTS.md"));
        assert!(prompt.contains("Be helpful."));
    }

    #[test]
    fn prompt_with_soul_file_adds_persona_instruction() {
        let mut params = base_params();
        params.context_files = vec![
            ContextFile {
                path: "SOUL.md".to_string(),
                content: "# Be a pirate".to_string(),
            },
        ];
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("embody its persona"));
        assert!(prompt.contains("Be a pirate"));
    }

    #[test]
    fn prompt_no_context_files_no_project_context_section() {
        let params = base_params();
        let prompt = build_system_prompt(&params);
        assert!(!prompt.contains("# Project Context"));
    }

    #[test]
    fn prompt_empty_tools_still_has_tooling_section() {
        let mut params = base_params();
        params.tool_names = vec![];
        params.tool_summaries = HashMap::new();
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("## Tooling"));
    }

    #[test]
    fn tools_without_summaries_listed_without_description() {
        let mut params = base_params();
        params.tool_names = vec!["custom_tool".to_string()];
        params.tool_summaries = HashMap::new();
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("- custom_tool"));
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p soloclaw prompt::tests -- --nocapture 2>&1 | head -20`
Expected: FAIL — `todo!()` panics.

**Step 3: Implement build_system_prompt**

Replace the `todo!()` in `build_system_prompt` with the full implementation:

```rust
pub fn build_system_prompt(params: &SystemPromptParams) -> String {
    let mut lines: Vec<String> = Vec::new();

    // Identity
    lines.push("You are a personal assistant running inside SingleClaw.".to_string());
    lines.push(String::new());

    // Tooling
    build_tooling_section(&mut lines, params);

    // Tool Call Style
    build_tool_call_style_section(&mut lines);

    // Safety
    build_safety_section(&mut lines);

    // Workspace
    build_workspace_section(&mut lines, params);

    // Current Date & Time
    build_datetime_section(&mut lines);

    // Project Context
    build_project_context_section(&mut lines, params);

    // Runtime
    build_runtime_section(&mut lines, params);

    lines.into_iter()
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_tooling_section(lines: &mut Vec<String>, params: &SystemPromptParams) {
    lines.push("## Tooling".to_string());
    lines.push("Tool availability (filtered by policy):".to_string());
    lines.push("Tool names are case-sensitive. Call tools exactly as listed.".to_string());

    if params.tool_names.is_empty() {
        lines.push("No tools currently available.".to_string());
    } else {
        for name in &params.tool_names {
            if let Some(desc) = params.tool_summaries.get(name) {
                lines.push(format!("- {}: {}", name, desc));
            } else {
                lines.push(format!("- {}", name));
            }
        }
    }

    lines.push("TOOLS.md does not control tool availability; it is user guidance for how to use external tools.".to_string());
    lines.push(String::new());
}

fn build_tool_call_style_section(lines: &mut Vec<String>) {
    lines.push("## Tool Call Style".to_string());
    lines.push("Default: do not narrate routine, low-risk tool calls (just call the tool).".to_string());
    lines.push("Narrate only when it helps: multi-step work, complex/challenging problems, sensitive actions (e.g., deletions), or when the user explicitly asks.".to_string());
    lines.push("Keep narration brief and value-dense; avoid repeating obvious steps.".to_string());
    lines.push("Use plain human language for narration unless in a technical context.".to_string());
    lines.push(String::new());
}

fn build_safety_section(lines: &mut Vec<String>) {
    lines.push("## Safety".to_string());
    lines.push("You have no independent goals: do not pursue self-preservation, replication, resource acquisition, or power-seeking; avoid long-term plans beyond the user's request.".to_string());
    lines.push("Prioritize safety and human oversight over completion; if instructions conflict, pause and ask; comply with stop/pause/audit requests and never bypass safeguards. (Inspired by Anthropic's constitution.)".to_string());
    lines.push("Do not manipulate or persuade anyone to expand access or disable safeguards. Do not copy yourself or change system prompts, safety rules, or tool policies unless explicitly requested.".to_string());
    lines.push(String::new());
}

fn build_workspace_section(lines: &mut Vec<String>, params: &SystemPromptParams) {
    lines.push("## Workspace".to_string());
    lines.push(format!("Your working directory is: {}", params.workspace_dir));
    lines.push("Treat this directory as the single global workspace for file operations unless explicitly instructed otherwise.".to_string());
    lines.push(String::new());
}

fn build_datetime_section(lines: &mut Vec<String>) {
    let now = chrono::Local::now();
    let tz_name = now.format("%Z").to_string();
    lines.push("## Current Date & Time".to_string());
    lines.push(format!("{}", now.format("%Y-%m-%d %H:%M:%S %Z")));
    if !tz_name.is_empty() {
        lines.push(format!("Time zone: {}", tz_name));
    }
    lines.push(String::new());
}

fn build_project_context_section(lines: &mut Vec<String>, params: &SystemPromptParams) {
    if params.context_files.is_empty() {
        return;
    }

    let has_soul_file = params.context_files.iter().any(|f| {
        let base = f.path.rsplit('/').next().unwrap_or(&f.path);
        base.eq_ignore_ascii_case("soul.md")
    });

    lines.push("# Project Context".to_string());
    lines.push(String::new());
    lines.push("The following project context files have been loaded:".to_string());

    if has_soul_file {
        lines.push("If SOUL.md is present, embody its persona and tone. Avoid stiff, generic replies; follow its guidance unless higher-priority instructions override it.".to_string());
    }

    lines.push(String::new());

    for file in &params.context_files {
        lines.push(format!("## {}", file.path));
        lines.push(String::new());
        lines.push(file.content.clone());
        lines.push(String::new());
    }
}

fn build_runtime_section(lines: &mut Vec<String>, params: &SystemPromptParams) {
    lines.push("## Runtime".to_string());

    let mut parts: Vec<String> = Vec::new();

    if !params.os.is_empty() {
        if !params.arch.is_empty() {
            parts.push(format!("os={} ({})", params.os, params.arch));
        } else {
            parts.push(format!("os={}", params.os));
        }
    } else if !params.arch.is_empty() {
        parts.push(format!("arch={}", params.arch));
    }

    if !params.model.is_empty() {
        parts.push(format!("model={}", params.model));
    }
    if !params.shell.is_empty() {
        parts.push(format!("shell={}", params.shell));
    }

    lines.push(format!("Runtime: {}", parts.join(" | ")));
}
```

**Step 4: Add chrono dependency**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo add chrono`

**Step 5: Run tests**

Run: `cargo test -p soloclaw prompt::tests -- --nocapture`
Expected: All 12 tests PASS.

**Step 6: Commit**

```bash
git add src/prompt.rs Cargo.toml Cargo.lock
git commit -m "feat: implement dynamic build_system_prompt() mirroring openclaw"
```

---

### Task 3: Implement load_context_files()

**Files:**
- Modify: `src/prompt.rs` (replace load_context_files todo!())

**Step 1: Add tests for load_context_files**

Add to the `#[cfg(test)]` module in `src/prompt.rs`:

```rust
    #[test]
    fn load_context_files_from_empty_dir() {
        let dir = std::env::temp_dir().join("soloclaw-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        let files = load_context_files(dir.to_str().unwrap());
        // No context files in a temp dir (unless someone put them there).
        // Just verify it doesn't panic and returns a vec.
        assert!(files.len() <= 4);
    }

    #[test]
    fn load_context_files_finds_soloclaw_md() {
        let dir = std::env::temp_dir().join("soloclaw-test-ctx");
        let _ = std::fs::create_dir_all(&dir);
        let ctx_path = dir.join(".soloclaw.md");
        std::fs::write(&ctx_path, "# Project notes\nSome context.").unwrap();

        let files = load_context_files(dir.to_str().unwrap());
        let found = files.iter().any(|f| f.path == ".soloclaw.md");
        assert!(found, "should find .soloclaw.md");

        // Cleanup
        let _ = std::fs::remove_file(&ctx_path);
    }
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p soloclaw prompt::tests::load_context -- --nocapture 2>&1 | head -10`
Expected: FAIL — `todo!()` panics.

**Step 3: Implement load_context_files**

Replace the `todo!()` in `load_context_files`:

```rust
pub fn load_context_files(workspace_dir: &str) -> Vec<ContextFile> {
    let dir = std::path::PathBuf::from(workspace_dir);
    let candidates = [".soloclaw.md", "SOUL.md", "AGENTS.md", "TOOLS.md"];
    let mut files = Vec::new();

    for name in &candidates {
        let path = dir.join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if !content.trim().is_empty() {
                files.push(ContextFile {
                    path: name.to_string(),
                    content,
                });
            }
        }
    }

    files
}
```

**Step 4: Run tests**

Run: `cargo test -p soloclaw prompt::tests -- --nocapture`
Expected: All tests PASS.

**Step 5: Commit**

```bash
git add src/prompt.rs
git commit -m "feat: implement load_context_files() for workspace context discovery"
```

---

### Task 4: Wire into app.rs — gather runtime info and build prompt

**Files:**
- Modify: `src/app.rs` (replace placeholder prompt with real build_system_prompt call)

**Step 1: Update app.rs to gather runtime info and build prompt**

In `src/app.rs`, update the import line:

```rust
use crate::prompt::{build_system_prompt, load_context_files, SystemPromptParams};
```

Replace the placeholder `system_prompt` assignment with:

```rust
// Gather runtime info and build the system prompt.
let workspace_dir = std::env::current_dir()
    .map(|p| p.to_string_lossy().to_string())
    .unwrap_or_else(|_| ".".to_string());

let context_files = load_context_files(&workspace_dir);

// Collect tool names and summaries from the registry.
let tool_defs = registry.to_definitions().await;
let tool_names: Vec<String> = tool_defs.iter().map(|d| d.name.clone()).collect();
let tool_summaries: std::collections::HashMap<String, String> = tool_defs
    .iter()
    .map(|d| (d.name.clone(), d.description.clone()))
    .collect();

let system_prompt = build_system_prompt(&SystemPromptParams {
    tool_names,
    tool_summaries,
    workspace_dir,
    os: std::env::consts::OS.to_string(),
    arch: std::env::consts::ARCH.to_string(),
    shell: std::env::var("SHELL").unwrap_or_default(),
    model: model.clone(),
    context_files,
});
```

**Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles cleanly.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: wire dynamic prompt builder into app startup with runtime info"
```

---

### Task 5: Rewrite integration tests

**Files:**
- Rewrite: `tests/prompt_integration.rs`

**Step 1: Rewrite the integration test file**

```rust
// ABOUTME: Integration tests for the dynamic system prompt builder.
// ABOUTME: Verifies prompt assembly from runtime parameters and context files.

use std::collections::HashMap;
use soloclaw::prompt::{build_system_prompt, load_context_files, ContextFile, SystemPromptParams};

fn base_params() -> SystemPromptParams {
    SystemPromptParams {
        tool_names: vec![
            "bash".to_string(),
            "read_file".to_string(),
            "write_file".to_string(),
            "list_files".to_string(),
            "search".to_string(),
        ],
        tool_summaries: {
            let mut m = HashMap::new();
            m.insert("bash".to_string(), "Execute a bash command and return its output.".to_string());
            m.insert("read_file".to_string(), "Read the contents of a file.".to_string());
            m.insert("write_file".to_string(), "Write content to a file.".to_string());
            m.insert("list_files".to_string(), "List files in a directory.".to_string());
            m.insert("search".to_string(), "Search for a pattern in files.".to_string());
            m
        },
        workspace_dir: "/home/user/project".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        shell: "/bin/bash".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        context_files: vec![],
    }
}

#[test]
fn full_prompt_has_all_runtime_sections() {
    let prompt = build_system_prompt(&base_params());

    // Identity
    assert!(prompt.contains("SingleClaw"), "missing identity");

    // Tooling
    assert!(prompt.contains("## Tooling"), "missing tooling section");
    assert!(prompt.contains("- bash: Execute a bash command"), "missing bash tool");

    // Tool Call Style
    assert!(prompt.contains("## Tool Call Style"), "missing tool call style");

    // Safety
    assert!(prompt.contains("## Safety"), "missing safety");

    // Workspace
    assert!(prompt.contains("## Workspace"), "missing workspace");
    assert!(prompt.contains("/home/user/project"), "missing workspace dir");

    // Date & Time
    assert!(prompt.contains("## Current Date & Time"), "missing datetime");

    // Runtime
    assert!(prompt.contains("## Runtime"), "missing runtime");
    assert!(prompt.contains("os=linux (x86_64)"), "missing os info");
    assert!(prompt.contains("model=claude-sonnet-4-20250514"), "missing model");
}

#[test]
fn prompt_without_context_files_has_no_project_context() {
    let prompt = build_system_prompt(&base_params());
    assert!(!prompt.contains("# Project Context"));
}

#[test]
fn prompt_with_soul_file_embodies_persona() {
    let mut params = base_params();
    params.context_files = vec![
        ContextFile {
            path: "SOUL.md".to_string(),
            content: "Be a friendly pirate who loves Rust.".to_string(),
        },
    ];
    let prompt = build_system_prompt(&params);
    assert!(prompt.contains("# Project Context"));
    assert!(prompt.contains("embody its persona"));
    assert!(prompt.contains("friendly pirate"));
}

#[test]
fn prompt_with_multiple_context_files() {
    let mut params = base_params();
    params.context_files = vec![
        ContextFile {
            path: ".soloclaw.md".to_string(),
            content: "This project uses React.".to_string(),
        },
        ContextFile {
            path: "AGENTS.md".to_string(),
            content: "Follow TDD.".to_string(),
        },
    ];
    let prompt = build_system_prompt(&params);
    assert!(prompt.contains("## .soloclaw.md"));
    assert!(prompt.contains("This project uses React."));
    assert!(prompt.contains("## AGENTS.md"));
    assert!(prompt.contains("Follow TDD."));
}

#[test]
fn all_five_builtin_tools_listed() {
    let prompt = build_system_prompt(&base_params());
    for tool in &["bash", "read_file", "write_file", "list_files", "search"] {
        assert!(prompt.contains(&format!("- {}:", tool)), "missing tool: {}", tool);
    }
}

#[test]
fn load_context_files_returns_empty_for_nonexistent_dir() {
    let files = load_context_files("/nonexistent/path/that/does/not/exist");
    assert!(files.is_empty());
}

#[test]
fn load_context_files_finds_files_in_workspace() {
    let dir = std::env::temp_dir().join("soloclaw-integration-ctx");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("SOUL.md"), "Be awesome.").unwrap();
    std::fs::write(dir.join(".soloclaw.md"), "Project notes.").unwrap();

    let files = load_context_files(dir.to_str().unwrap());
    assert!(files.iter().any(|f| f.path == "SOUL.md"));
    assert!(files.iter().any(|f| f.path == ".soloclaw.md"));

    // Cleanup
    let _ = std::fs::remove_file(dir.join("SOUL.md"));
    let _ = std::fs::remove_file(dir.join(".soloclaw.md"));
}

#[test]
fn section_order_matches_openclaw() {
    let prompt = build_system_prompt(&base_params());

    let identity_pos = prompt.find("SingleClaw").unwrap();
    let tooling_pos = prompt.find("## Tooling").unwrap();
    let style_pos = prompt.find("## Tool Call Style").unwrap();
    let safety_pos = prompt.find("## Safety").unwrap();
    let workspace_pos = prompt.find("## Workspace").unwrap();
    let datetime_pos = prompt.find("## Current Date & Time").unwrap();
    let runtime_pos = prompt.find("## Runtime").unwrap();

    assert!(identity_pos < tooling_pos, "identity before tooling");
    assert!(tooling_pos < style_pos, "tooling before style");
    assert!(style_pos < safety_pos, "style before safety");
    assert!(safety_pos < workspace_pos, "safety before workspace");
    assert!(workspace_pos < datetime_pos, "workspace before datetime");
    assert!(datetime_pos < runtime_pos, "datetime before runtime");
}
```

**Step 2: Run integration tests**

Run: `cargo test --test prompt_integration -- --nocapture`
Expected: All 8 tests PASS.

**Step 3: Run full test suite**

Run: `cargo test`
Expected: All tests pass.

**Step 4: Commit**

```bash
git add tests/prompt_integration.rs
git commit -m "test: rewrite integration tests for dynamic prompt builder"
```

---

### Task 6: Final verification and cleanup

**Files:** None (verification only)

**Step 1: Run full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass.

**Step 2: Run clippy**

Run: `cargo clippy 2>&1 | grep "soloclaw"`
Expected: No warnings from soloclaw code.

**Step 3: Verify binary builds**

Run: `cargo build 2>&1 | tail -5`
Expected: Compiles successfully.

**Step 4: Verify old prompts directory is gone**

Run: `ls src/prompts/ 2>&1`
Expected: "No such file or directory"

**Step 5: Verify git log is clean**

Run: `git log --oneline feat/layered-system-prompt --not main`
Expected: Clean commit history.
