# Dynamic System Prompt Builder Design

**Date**: 2026-02-16
**Status**: Approved

## Summary

Replace soloclaw's layered SystemPromptBuilder (compiled-in defaults + overrides)
with a dynamic prompt builder that faithfully ports openclaw's `buildAgentSystemPrompt()`.
The prompt is assembled at runtime from capabilities, environment, and workspace context files.

## Architecture

Single `build_system_prompt(params: &SystemPromptParams) -> String` function that mirrors
openclaw's approach: takes runtime info, builds sections conditionally, returns assembled prompt.

### Input

```rust
pub struct SystemPromptParams {
    pub tool_names: Vec<String>,
    pub tool_summaries: HashMap<String, String>,
    pub workspace_dir: String,
    pub os: String,
    pub arch: String,
    pub shell: String,
    pub model: String,
    pub context_files: Vec<ContextFile>,
}

pub struct ContextFile {
    pub path: String,
    pub content: String,
}
```

### Sections (in order, mirroring openclaw)

1. Identity — "You are a personal assistant running inside SingleClaw."
2. Tooling — Dynamic tool list from Registry with descriptions
3. Tool Call Style — Narration guidance (port verbatim from openclaw)
4. Safety — Autonomy constraints (port verbatim from openclaw)
5. Workspace — Working directory + guidance
6. Current Date & Time — Timezone-aware timestamp
7. Project Context — Injected workspace files (SOUL.md, AGENTS.md, TOOLS.md, .soloclaw.md)
8. Runtime — os, arch, model, shell info line

### Context File Loading

Search cwd for: `.soloclaw.md`, `SOUL.md`, `AGENTS.md`, `TOOLS.md`
If SOUL.md found, add "embody its persona" instruction (matching openclaw).
No compiled-in defaults — if no files exist, prompt is just runtime sections.

### Deletions

- src/prompts/ directory and all files
- Old SystemPromptBuilder struct

### Changes

- src/prompt.rs — full rewrite
- src/app.rs — gather runtime info, load context files, call build_system_prompt()
- tests/prompt_integration.rs — rewrite for new API
