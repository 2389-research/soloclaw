// ABOUTME: Configuration loading and setup for soloclaw.
// ABOUTME: Reads XDG config files, supports legacy path fallback, and provides interactive setup.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::Deserialize;

use mux::prelude::*;

use crate::approval::ApprovalsFile;

const APP_NAME: &str = "soloclaw";

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub llm: LlmConfig,
    pub approval: ApprovalConfig,
    pub permissions: PermissionsConfig,
    pub skills: SkillsConfig,
    pub compaction: CompactionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            llm: LlmConfig::default(),
            approval: ApprovalConfig::default(),
            permissions: PermissionsConfig::default(),
            skills: SkillsConfig::default(),
            compaction: CompactionConfig::default(),
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
    pub openai: ProviderConfig,
    pub anthropic: ProviderConfig,
    pub gemini: ProviderConfig,
    pub openrouter: ProviderConfig,
    pub ollama: OllamaConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-5-20250929".to_string(),
            max_tokens: 4096,
            openai: ProviderConfig::default(),
            anthropic: ProviderConfig::default(),
            gemini: ProviderConfig::default(),
            openrouter: ProviderConfig::default(),
            ollama: OllamaConfig::default(),
        }
    }
}

/// Shared provider configuration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url: Option<String>,
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

/// Runtime permission toggles.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PermissionsConfig {
    /// If true, bypasses all approval checks and executes tool calls directly.
    pub bypass_approvals: bool,
}

impl Default for PermissionsConfig {
    fn default() -> Self {
        Self {
            bypass_approvals: false,
        }
    }
}

/// Compaction configuration for automatic conversation summarization.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    /// Whether automatic compaction is enabled.
    pub enabled: bool,
    /// Override token limit triggering compaction (default: 90% of context window).
    pub threshold_token_limit: Option<u64>,
    /// Maximum tokens allocated for retained user messages after compaction.
    pub user_message_budget_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        use crate::agent::compaction::DEFAULT_USER_MESSAGE_BUDGET_TOKENS;
        Self {
            enabled: true,
            threshold_token_limit: None,
            user_message_budget_tokens: DEFAULT_USER_MESSAGE_BUDGET_TOKENS,
        }
    }
}

/// Skill prompt loading configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SkillsConfig {
    /// Enable loading SKILL.md files into the system prompt.
    pub enabled: bool,
    /// Include XDG config skills from $XDG_CONFIG_HOME/soloclaw/skills.
    pub include_xdg_config: bool,
    /// Include workspace-level skills from ./skills.
    pub include_workspace: bool,
    /// Include personal skills from ~/.agents/skills.
    pub include_agents_home: bool,
    /// Include Codex skills from $CODEX_HOME/skills or ~/.codex/skills.
    pub include_codex_home: bool,
    /// Maximum number of skill files to include.
    pub max_files: usize,
    /// Maximum file size in bytes for each SKILL.md.
    pub max_file_bytes: usize,
    /// Maximum total characters across all included skill contents.
    pub max_total_chars: usize,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            include_xdg_config: true,
            include_workspace: true,
            include_agents_home: true,
            include_codex_home: true,
            max_files: 24,
            max_file_bytes: 128 * 1024,
            max_total_chars: 32_000,
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
    /// Load config from XDG config path, falling back to legacy path and then defaults.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::resolved_config_path();
        if !path.exists() {
            let xdg_path = Self::config_path();
            if let Some(parent) = xdg_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&xdg_path, default_config_toml())?;
            let content = std::fs::read_to_string(&xdg_path)?;
            let config: Self = toml::from_str(&content)?;
            return Ok(config);
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Path to the XDG config directory for soloclaw.
    pub fn config_dir() -> PathBuf {
        if let Ok(xdg_home) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg_home).join(APP_NAME);
        }

        if let Some(base) = dirs::config_dir() {
            return base.join(APP_NAME);
        }

        if let Some(home) = dirs::home_dir() {
            return home.join(".config").join(APP_NAME);
        }

        PathBuf::from(".").join(APP_NAME)
    }

    /// Legacy config directory used before XDG migration.
    pub fn legacy_config_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(format!(".{}", APP_NAME))
    }

    /// Path to the config file.
    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    /// Path to the approvals file.
    pub fn approvals_path() -> PathBuf {
        Self::config_dir().join("approvals.json")
    }

    /// Path to provider secrets loaded as dotenv env vars.
    pub fn secrets_env_path() -> PathBuf {
        Self::config_dir().join("secrets.env")
    }

    /// Path to the XDG data directory for soloclaw.
    pub fn data_dir() -> PathBuf {
        if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg_data).join(APP_NAME);
        }
        if let Some(base) = dirs::data_dir() {
            return base.join(APP_NAME);
        }
        if let Some(home) = dirs::home_dir() {
            return home.join(".local").join("share").join(APP_NAME);
        }
        PathBuf::from(".").join(APP_NAME)
    }

    /// Path to the sessions directory inside the data directory.
    pub fn sessions_dir() -> PathBuf {
        Self::data_dir().join("sessions")
    }

    fn resolved_config_path() -> PathBuf {
        let xdg = Self::config_path();
        if xdg.exists() {
            return xdg;
        }

        let legacy = Self::legacy_config_dir().join("config.toml");
        if legacy.exists() {
            return legacy;
        }

        xdg
    }
}

/// Recommended default model for each provider.
pub fn default_model_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai" => "gpt-5.2",
        "anthropic" => "claude-sonnet-4-5-20250929",
        "gemini" => "gemini-2.5-pro",
        "openrouter" => "anthropic/claude-sonnet-4",
        "ollama" => "llama3.2",
        _ => "claude-sonnet-4-5-20250929",
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

/// Interactive setup command: initializes XDG config and provider secrets.
pub fn run_setup() -> anyhow::Result<()> {
    let config_dir = Config::config_dir();
    std::fs::create_dir_all(&config_dir)?;
    let skills_dir = config_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let config_path = Config::config_path();
    if !config_path.exists() {
        std::fs::write(&config_path, default_config_toml())?;
        println!("Created {}", config_path.display());
    } else {
        println!("Using existing {}", config_path.display());
    }

    let approvals_path = Config::approvals_path();
    if !approvals_path.exists() {
        let approvals = serde_json::to_string_pretty(&ApprovalsFile::default())?;
        std::fs::write(&approvals_path, approvals)?;
        println!("Created {}", approvals_path.display());
    } else {
        println!("Using existing {}", approvals_path.display());
    }

    let secrets_path = Config::secrets_env_path();
    let mut env_map = load_env_file(&secrets_path)?;
    configure_provider_keys(&mut env_map)?;
    write_env_file(&secrets_path, &env_map)?;

    let skills_readme = skills_dir.join("README.md");
    if !skills_readme.exists() {
        std::fs::write(&skills_readme, default_skills_readme())?;
        println!("Created {}", skills_readme.display());
    } else {
        println!("Using existing {}", skills_readme.display());
    }

    println!("Wrote {}", secrets_path.display());
    println!("Setup complete.");
    println!("Run: claw");

    Ok(())
}

fn configure_provider_keys(env_map: &mut HashMap<String, String>) -> anyhow::Result<()> {
    let keys = [
        ("ANTHROPIC_API_KEY", "Anthropic"),
        ("OPENAI_API_KEY", "OpenAI"),
        ("GEMINI_API_KEY", "Google Gemini"),
        ("OPENROUTER_API_KEY", "OpenRouter"),
    ];

    println!();
    println!("Configure AI provider keys (leave blank to skip):");
    for (key, provider_name) in keys {
        let existing = env_map.get(key).cloned().unwrap_or_default();
        let prompt = if existing.is_empty() {
            format!("{provider_name} ({key}): ")
        } else {
            format!("{provider_name} ({key}) [existing set, Enter to keep]: ")
        };

        let input = prompt_line(&prompt)?;
        let trimmed = input.trim();
        if trimmed.is_empty() {
            continue;
        }
        env_map.insert(key.to_string(), trimmed.to_string());
    }

    Ok(())
}

fn prompt_line(prompt: &str) -> anyhow::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input)
}

fn load_env_file(path: &PathBuf) -> anyhow::Result<HashMap<String, String>> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    Ok(map)
}

fn write_env_file(path: &PathBuf, env_map: &HashMap<String, String>) -> anyhow::Result<()> {
    let mut keys = env_map.keys().cloned().collect::<Vec<_>>();
    keys.sort();

    let mut out = String::from("# soloclaw provider secrets\n");
    for key in keys {
        if let Some(value) = env_map.get(&key) {
            out.push_str(&format!("{}={}\n", key, value));
        }
    }

    std::fs::write(path, out)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }

    Ok(())
}

fn default_config_toml() -> String {
    r#"[llm]
provider = "anthropic"
model = "claude-sonnet-4-5-20250929"
max_tokens = 4096

[llm.openai]
base_url = "https://api.openai.com/v1"
# Recommended: model = "gpt-5.2"

[llm.anthropic]
base_url = "https://api.anthropic.com"

[llm.gemini]
base_url = "https://generativelanguage.googleapis.com/v1beta"

[llm.openrouter]
base_url = "https://openrouter.ai/api/v1"

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "allowlist"
ask = "on-miss"
ask_fallback = "deny"
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
max_file_bytes = 131072
max_total_chars = 32000

[compaction]
enabled = true
# threshold_token_limit = 180000
user_message_budget_tokens = 20000
"#
    .to_string()
}

fn default_skills_readme() -> &'static str {
    r#"# soloclaw skills

Put skill folders here.

Each skill should have this layout:

```text
skills/
  my-skill/
    SKILL.md
```

`soloclaw` will load `SKILL.md` files from this directory into the system prompt
when `skills.enabled` and `skills.include_xdg_config` are true in `config.toml`.
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.max_tokens, 4096);
        assert!(config.llm.openai.base_url.is_none());
        assert_eq!(config.approval.timeout_seconds, 120);
        assert!(!config.permissions.bypass_approvals);
        assert!(config.skills.enabled);
        assert!(config.skills.include_xdg_config);
        assert_eq!(config.skills.max_files, 24);
    }

    #[test]
    fn parse_config_toml() {
        let toml_str = r#"
[llm]
provider = "ollama"
model = "llama3"
max_tokens = 2048

[llm.openai]
base_url = "https://example-openai/v1"

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "full"
ask = "always"
timeout_seconds = 60

[permissions]
bypass_approvals = true

[skills]
enabled = true
include_xdg_config = true
include_workspace = false
max_files = 5
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "ollama");
        assert_eq!(config.llm.model, "llama3");
        assert_eq!(config.llm.max_tokens, 2048);
        assert_eq!(
            config.llm.openai.base_url.as_deref(),
            Some("https://example-openai/v1")
        );
        assert_eq!(config.approval.security, "full");
        assert_eq!(config.approval.ask, "always");
        assert_eq!(config.approval.timeout_seconds, 60);
        assert!(config.permissions.bypass_approvals);
        assert!(config.skills.enabled);
        assert!(config.skills.include_xdg_config);
        assert!(!config.skills.include_workspace);
        assert_eq!(config.skills.max_files, 5);
    }

    #[test]
    fn sessions_dir_is_subpath_of_data_dir() {
        let data_dir = Config::data_dir();
        let sessions_dir = Config::sessions_dir();
        assert!(
            sessions_dir.starts_with(&data_dir),
            "sessions_dir {:?} should start with data_dir {:?}",
            sessions_dir,
            data_dir
        );
        assert_eq!(sessions_dir, data_dir.join("sessions"));
    }

    #[test]
    fn data_dir_contains_app_name() {
        let data_dir = Config::data_dir();
        let dir_str = data_dir.to_string_lossy();
        assert!(
            dir_str.contains("soloclaw"),
            "data_dir {:?} should contain 'soloclaw'",
            data_dir
        );
    }

    #[test]
    fn parse_partial_config_uses_defaults() {
        let toml_str = r#"
[llm]
provider = "openai"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.llm.model, "claude-sonnet-4-5-20250929");
        assert_eq!(config.approval.timeout_seconds, 120);
        assert!(!config.permissions.bypass_approvals);
        assert!(config.skills.enabled);
    }

    #[test]
    fn compaction_config_has_correct_defaults() {
        let config = CompactionConfig::default();
        assert!(config.enabled);
        assert!(config.threshold_token_limit.is_none());
        assert_eq!(config.user_message_budget_tokens, 20_000);
    }

    #[test]
    fn compaction_config_parsed_from_toml() {
        let toml_str = r#"
[compaction]
enabled = false
threshold_token_limit = 100000
user_message_budget_tokens = 10000
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(!config.compaction.enabled);
        assert_eq!(config.compaction.threshold_token_limit, Some(100_000));
        assert_eq!(config.compaction.user_message_budget_tokens, 10_000);
    }

    #[test]
    fn default_config_includes_compaction_defaults() {
        let config = Config::default();
        assert!(config.compaction.enabled);
        assert!(config.compaction.threshold_token_limit.is_none());
        assert_eq!(config.compaction.user_message_budget_tokens, 20_000);
    }
}
