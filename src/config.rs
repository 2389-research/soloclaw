// ABOUTME: Configuration loading for simpleclaw.
// ABOUTME: Reads ~/.simpleclaw/config.toml, .mcp.json, and CLI overrides.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

use mux::prelude::*;

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub llm: LlmConfig,
    pub approval: ApprovalConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            llm: LlmConfig::default(),
            approval: ApprovalConfig::default(),
        }
    }
}

/// LLM provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub ollama: OllamaConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            ollama: OllamaConfig::default(),
        }
    }
}

/// Ollama-specific configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub base_url: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
        }
    }
}

/// Approval defaults in config.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApprovalConfig {
    pub security: String,
    pub ask: String,
    pub ask_fallback: String,
    pub timeout_seconds: u64,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            security: "allowlist".to_string(),
            ask: "on-miss".to_string(),
            ask_fallback: "deny".to_string(),
            timeout_seconds: 120,
        }
    }
}

/// MCP server configuration from .mcp.json.
#[derive(Debug, Deserialize)]
struct McpConfigFile {
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Deserialize)]
struct McpServerEntry {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

impl Config {
    /// Load config from ~/.simpleclaw/config.toml, falling back to defaults.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Path to the config file.
    pub fn config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".simpleclaw")
            .join("config.toml")
    }

    /// Path to the approvals file.
    pub fn approvals_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".simpleclaw")
            .join("approvals.json")
    }
}

/// Load MCP server configs from .mcp.json.
pub fn load_mcp_configs() -> anyhow::Result<Vec<McpServerConfig>> {
    let path = find_mcp_config();
    let Some(path) = path else {
        return Ok(vec![]);
    };

    let content = std::fs::read_to_string(&path)?;
    let config: McpConfigFile = serde_json::from_str(&content)?;

    let servers = config
        .mcp_servers
        .into_iter()
        .map(|(name, entry)| McpServerConfig {
            name,
            transport: McpTransport::Stdio {
                command: entry.command,
                args: entry.args,
                env: entry.env,
            },
        })
        .collect();

    Ok(servers)
}

fn find_mcp_config() -> Option<PathBuf> {
    let local = PathBuf::from(".mcp.json");
    if local.exists() {
        return Some(local);
    }

    if let Some(home) = dirs::home_dir() {
        let global = home.join(".mcp.json");
        if global.exists() {
            return Some(global);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.max_tokens, 4096);
        assert_eq!(config.approval.timeout_seconds, 120);
    }

    #[test]
    fn parse_config_toml() {
        let toml_str = r#"
[llm]
provider = "ollama"
model = "llama3"
max_tokens = 2048

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "full"
ask = "always"
timeout_seconds = 60
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "ollama");
        assert_eq!(config.llm.model, "llama3");
        assert_eq!(config.llm.max_tokens, 2048);
        assert_eq!(config.approval.security, "full");
        assert_eq!(config.approval.ask, "always");
        assert_eq!(config.approval.timeout_seconds, 60);
    }

    #[test]
    fn parse_partial_config_uses_defaults() {
        let toml_str = r#"
[llm]
provider = "openai"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.llm.model, "claude-sonnet-4-20250514");
        assert_eq!(config.approval.timeout_seconds, 120);
    }
}
