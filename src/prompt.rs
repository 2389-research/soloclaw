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
pub fn build_system_prompt(_params: &SystemPromptParams) -> String {
    // Placeholder — implemented in Task 2.
    "You are a personal assistant running inside SimpleClaw.".to_string()
}

/// Load context files from the workspace directory.
///
/// Searches for: .simpleclaw.md, SOUL.md, AGENTS.md, TOOLS.md
pub fn load_context_files(_workspace_dir: &str) -> Vec<ContextFile> {
    // Placeholder — implemented in Task 3.
    Vec::new()
}
