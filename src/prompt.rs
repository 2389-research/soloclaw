// ABOUTME: Layered system prompt builder — assembles soul/agents/tools into one prompt.
// ABOUTME: Compiles defaults from src/prompts/*.md, supports file-based overrides.

use std::fs;
use std::path::PathBuf;

/// Compiled-in default prompt layers.
const DEFAULT_SOUL: &str = include_str!("prompts/soul.md");
const DEFAULT_AGENTS: &str = include_str!("prompts/agents.md");
const DEFAULT_TOOLS: &str = include_str!("prompts/tools.md");

/// Reads a file if it exists, returning None otherwise.
pub fn read_if_exists(path: PathBuf) -> Option<String> {
    if path.exists() {
        fs::read_to_string(&path).ok()
    } else {
        None
    }
}

/// Assembles a system prompt from layered sources: soul, agents, tools, and
/// an optional local override. Each layer can be replaced by user files in
/// `~/.simpleclaw/` or augmented with a `.simpleclaw.md` in the working directory.
#[derive(Debug, Clone)]
pub struct SystemPromptBuilder {
    pub soul: String,
    pub agents: String,
    pub tools: String,
    pub local: Option<String>,
}

impl SystemPromptBuilder {
    /// Creates a new builder loaded with the compiled-in defaults.
    pub fn new() -> Self {
        Self {
            soul: DEFAULT_SOUL.to_string(),
            agents: DEFAULT_AGENTS.to_string(),
            tools: DEFAULT_TOOLS.to_string(),
            local: None,
        }
    }

    /// Checks `~/.simpleclaw/` for override files and replaces layers if found.
    pub fn load_overrides(&mut self) -> &mut Self {
        let base = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".simpleclaw");

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

    /// Checks for `.simpleclaw.md` in the current working directory and sets `local`.
    pub fn load_local(&mut self) -> &mut Self {
        let path = PathBuf::from(".simpleclaw.md");
        self.local = read_if_exists(path);
        self
    }

    /// Concatenates all non-empty layers separated by `"\n\n"`.
    pub fn build(&self) -> String {
        let layers: Vec<&str> = [
            Some(self.soul.as_str()),
            Some(self.agents.as_str()),
            Some(self.tools.as_str()),
            self.local.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();

        layers.join("\n\n")
    }
}

impl Default for SystemPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_contain_soul_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(
            prompt.contains("genuinely helpful"),
            "expected soul content 'genuinely helpful' in prompt"
        );
    }

    #[test]
    fn defaults_contain_agents_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(
            prompt.contains("Agent Guidelines"),
            "expected agents content 'Agent Guidelines' in prompt"
        );
    }

    #[test]
    fn defaults_contain_tools_content() {
        let builder = SystemPromptBuilder::new();
        let prompt = builder.build();
        assert!(
            prompt.contains("Local Notes"),
            "expected tools content 'Local Notes' in prompt"
        );
    }

    #[test]
    fn build_skips_empty_layers() {
        let mut builder = SystemPromptBuilder::new();
        builder.tools = String::new();
        let prompt = builder.build();
        assert!(
            !prompt.contains("\n\n\n\n"),
            "empty layer should not produce quadruple newlines"
        );
    }

    #[test]
    fn local_layer_appended_when_present() {
        let mut builder = SystemPromptBuilder::new();
        builder.local = Some("project-specific instructions here".to_string());
        let prompt = builder.build();
        assert!(
            prompt.contains("project-specific instructions here"),
            "local layer should appear in built prompt"
        );
    }

    #[test]
    fn override_replaces_layer() {
        let mut builder = SystemPromptBuilder::new();
        builder.soul = "custom soul content for testing".to_string();
        let prompt = builder.build();
        assert!(
            prompt.contains("custom soul content for testing"),
            "custom soul should appear in prompt"
        );
        assert!(
            !prompt.contains("genuinely helpful"),
            "default soul should be replaced, not present"
        );
    }
}
