// ABOUTME: Dynamic system prompt builder — assembles prompt from runtime capabilities.
// ABOUTME: Faithful port of openclaw's buildAgentSystemPrompt() pattern.

use std::collections::HashMap;
use std::path::PathBuf;

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
    let mut lines: Vec<String> = Vec::new();

    // Identity
    lines.push("You are a personal assistant running inside SimpleClaw.".to_string());
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

    // Project Context (only if context files exist)
    build_project_context_section(&mut lines, params);

    // Runtime
    build_runtime_section(&mut lines, params);

    lines.join("\n")
}

/// Load context files from the workspace directory.
///
/// Searches for: .simpleclaw.md, SOUL.md, AGENTS.md, TOOLS.md
/// Skips files that don't exist or are empty.
pub fn load_context_files(workspace_dir: &str) -> Vec<ContextFile> {
    let dir = PathBuf::from(workspace_dir);
    let candidates = [".simpleclaw.md", "SOUL.md", "AGENTS.md", "TOOLS.md"];
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

    lines.push(
        "TOOLS.md does not control tool availability; it is user guidance for how to use external tools."
            .to_string(),
    );
    lines.push(String::new());
}

fn build_tool_call_style_section(lines: &mut Vec<String>) {
    lines.push("## Tool Call Style".to_string());
    lines.push(
        "Default: do not narrate routine, low-risk tool calls (just call the tool).".to_string(),
    );
    lines.push(
        "Narrate only when it helps: multi-step work, complex/challenging problems, sensitive actions (e.g., deletions), or when the user explicitly asks.".to_string(),
    );
    lines.push(
        "Keep narration brief and value-dense; avoid repeating obvious steps.".to_string(),
    );
    lines.push(
        "Use plain human language for narration unless in a technical context.".to_string(),
    );
    lines.push(String::new());
}

fn build_safety_section(lines: &mut Vec<String>) {
    lines.push("## Safety".to_string());
    lines.push(
        "You have no independent goals: do not pursue self-preservation, replication, resource acquisition, or power-seeking; avoid long-term plans beyond the user's request.".to_string(),
    );
    lines.push(
        "Prioritize safety and human oversight over completion; if instructions conflict, pause and ask; comply with stop/pause/audit requests and never bypass safeguards. (Inspired by Anthropic's constitution.)".to_string(),
    );
    lines.push(
        "Do not manipulate or persuade anyone to expand access or disable safeguards. Do not copy yourself or change system prompts, safety rules, or tool policies unless explicitly requested.".to_string(),
    );
    lines.push(String::new());
}

fn build_workspace_section(lines: &mut Vec<String>, params: &SystemPromptParams) {
    lines.push("## Workspace".to_string());
    lines.push(format!(
        "Your working directory is: {}",
        params.workspace_dir
    ));
    lines.push(
        "Treat this directory as the single global workspace for file operations unless explicitly instructed otherwise.".to_string(),
    );
    lines.push(String::new());
}

fn build_datetime_section(lines: &mut Vec<String>) {
    let now = chrono::Local::now();
    lines.push("## Current Date & Time".to_string());
    lines.push(format!("{}", now.format("%Y-%m-%d %H:%M:%S %Z")));
    lines.push(format!("Time zone: {}", now.format("%Z")));
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
        lines.push(
            "If SOUL.md is present, embody its persona and tone. Avoid stiff, generic replies; follow its guidance unless higher-priority instructions override it.".to_string(),
        );
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
        assert!(prompt.starts_with("You are a personal assistant running inside SimpleClaw."));
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
        assert!(prompt.contains("do not narrate routine"));
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
        assert!(prompt.contains("Time zone:"));
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
        params.context_files = vec![ContextFile {
            path: "AGENTS.md".to_string(),
            content: "# My Guidelines\nBe helpful.".to_string(),
        }];
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("# Project Context"));
        assert!(prompt.contains("## AGENTS.md"));
        assert!(prompt.contains("Be helpful."));
    }

    #[test]
    fn prompt_with_soul_file_adds_persona_instruction() {
        let mut params = base_params();
        params.context_files = vec![ContextFile {
            path: "SOUL.md".to_string(),
            content: "# Be a pirate".to_string(),
        }];
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
        assert!(prompt.contains("No tools currently available."));
    }

    #[test]
    fn tools_without_summaries_listed_without_description() {
        let mut params = base_params();
        params.tool_names = vec!["custom_tool".to_string()];
        params.tool_summaries = HashMap::new();
        let prompt = build_system_prompt(&params);
        assert!(prompt.contains("- custom_tool"));
        assert!(!prompt.contains("- custom_tool:"));
    }

    #[test]
    fn load_context_files_from_nonexistent_dir() {
        let files = load_context_files("/nonexistent/path/that/does/not/exist");
        assert!(files.is_empty());
    }

    #[test]
    fn load_context_files_finds_files() {
        let dir = std::env::temp_dir().join("simpleclaw-test-ctx-2");
        let _ = std::fs::create_dir_all(&dir);
        let ctx_path = dir.join(".simpleclaw.md");
        std::fs::write(&ctx_path, "# Project notes\nSome context.").unwrap();

        let files = load_context_files(dir.to_str().unwrap());
        let found = files.iter().any(|f| f.path == ".simpleclaw.md");
        assert!(found, "should find .simpleclaw.md");

        let _ = std::fs::remove_file(&ctx_path);
    }

    #[test]
    fn load_context_files_skips_empty_files() {
        let dir = std::env::temp_dir().join("simpleclaw-test-ctx-empty");
        let _ = std::fs::create_dir_all(&dir);
        let ctx_path = dir.join("SOUL.md");
        std::fs::write(&ctx_path, "   \n  ").unwrap();

        let files = load_context_files(dir.to_str().unwrap());
        let found = files.iter().any(|f| f.path == "SOUL.md");
        assert!(!found, "should skip empty SOUL.md");

        let _ = std::fs::remove_file(&ctx_path);
    }

    #[test]
    fn section_order_matches_openclaw() {
        let prompt = build_system_prompt(&base_params());

        let identity_pos = prompt.find("SimpleClaw").unwrap();
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
}
