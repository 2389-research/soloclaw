// ABOUTME: Persistent allowlist storage with glob pattern matching.
// ABOUTME: JSON-backed tool approval configs, wildcard fallback, and duplicate-safe entry management.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use glob::Pattern;
use serde::{Deserialize, Serialize};

use super::types::ToolSecurity;

/// A single allowlist entry recording a permitted pattern and usage metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistEntry {
    /// Glob pattern that matches tool invocation arguments (e.g. a resolved path).
    pub pattern: String,
    /// When this entry was added.
    pub added_at: DateTime<Utc>,
    /// When this entry was last matched against an invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    /// The command string that last matched this entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used_command: Option<String>,
}

/// Per-tool security configuration paired with its allowlist entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalConfig {
    /// Security policy for this tool (flattened so security/ask/ask_fallback appear inline).
    #[serde(flatten)]
    pub security: ToolSecurity,
    /// Allowlisted patterns for this tool.
    #[serde(default)]
    pub allowlist: Vec<AllowlistEntry>,
}

/// Top-level approvals file that persists to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalsFile {
    /// Schema version for forward compatibility.
    pub version: u32,
    /// Default security applied when no tool-specific config exists.
    pub defaults: ToolSecurity,
    /// Per-tool overrides keyed by tool name (supports "*" wildcard).
    #[serde(default)]
    pub tools: HashMap<String, ToolApprovalConfig>,
}

impl Default for ApprovalsFile {
    fn default() -> Self {
        Self {
            version: 1,
            defaults: ToolSecurity::default(),
            tools: HashMap::new(),
        }
    }
}

impl ApprovalsFile {
    /// Load an approvals file from disk. Returns defaults if the file doesn't exist.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let file: Self = serde_json::from_str(&content)?;
        Ok(file)
    }

    /// Save the approvals file to disk, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get the security configuration for a tool by name.
    ///
    /// Lookup order: exact tool name → "*" wildcard → defaults.
    pub fn tool_security(&self, tool_name: &str) -> &ToolSecurity {
        if let Some(config) = self.tools.get(tool_name) {
            return &config.security;
        }
        if let Some(wildcard) = self.tools.get("*") {
            return &wildcard.security;
        }
        &self.defaults
    }

    /// Check if a pattern matches any allowlist entry for the given tool.
    ///
    /// Each stored entry pattern is compiled as a glob and tested against the input.
    pub fn is_allowed(&self, tool_name: &str, pattern: &str) -> bool {
        let Some(config) = self.tools.get(tool_name) else {
            return false;
        };
        config.allowlist.iter().any(|entry| {
            Pattern::new(&entry.pattern)
                .map(|p| p.matches(pattern))
                .unwrap_or(false)
        })
    }

    /// Add a pattern to a tool's allowlist, skipping if the exact pattern already exists.
    ///
    /// Creates the tool config with default security if it doesn't exist yet.
    pub fn add_to_allowlist(&mut self, tool_name: &str, pattern: &str) {
        let config = self
            .tools
            .entry(tool_name.to_string())
            .or_insert_with(|| ToolApprovalConfig {
                security: self.defaults.clone(),
                allowlist: Vec::new(),
            });

        // Skip duplicates.
        if config.allowlist.iter().any(|e| e.pattern == pattern) {
            return;
        }

        config.allowlist.push(AllowlistEntry {
            pattern: pattern.to_string(),
            added_at: Utc::now(),
            last_used_at: None,
            last_used_command: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::types::{AskMode, SecurityLevel};

    #[test]
    fn default_approvals_file() {
        let file = ApprovalsFile::default();
        assert_eq!(file.version, 1);
        assert_eq!(file.defaults.security, SecurityLevel::Allowlist);
        assert_eq!(file.defaults.ask, AskMode::OnMiss);
        assert!(file.tools.is_empty());
    }

    #[test]
    fn tool_security_falls_back_to_defaults() {
        let file = ApprovalsFile::default();
        let sec = file.tool_security("nonexistent");
        assert_eq!(sec.security, SecurityLevel::Allowlist);
    }

    #[test]
    fn tool_security_uses_specific_config() {
        let mut file = ApprovalsFile::default();
        file.tools.insert(
            "bash".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Full,
                    ask: AskMode::Off,
                    ..ToolSecurity::default()
                },
                allowlist: Vec::new(),
            },
        );
        let sec = file.tool_security("bash");
        assert_eq!(sec.security, SecurityLevel::Full);
        assert_eq!(sec.ask, AskMode::Off);
    }

    #[test]
    fn tool_security_wildcard_fallback() {
        let mut file = ApprovalsFile::default();
        file.tools.insert(
            "*".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Deny,
                    ask: AskMode::Always,
                    ..ToolSecurity::default()
                },
                allowlist: Vec::new(),
            },
        );
        // Unknown tool falls through to wildcard.
        let sec = file.tool_security("unknown_tool");
        assert_eq!(sec.security, SecurityLevel::Deny);
        assert_eq!(sec.ask, AskMode::Always);
    }

    #[test]
    fn allowlist_exact_match() {
        let mut file = ApprovalsFile::default();
        file.add_to_allowlist("bash", "/usr/bin/ls");
        assert!(file.is_allowed("bash", "/usr/bin/ls"));
        assert!(!file.is_allowed("bash", "/usr/bin/rm"));
    }

    #[test]
    fn allowlist_glob_match() {
        let mut file = ApprovalsFile::default();
        file.add_to_allowlist("bash", "/usr/bin/*");
        assert!(file.is_allowed("bash", "/usr/bin/ls"));
        assert!(file.is_allowed("bash", "/usr/bin/cat"));
        assert!(!file.is_allowed("bash", "/usr/local/bin/ls"));
    }

    #[test]
    fn allowlist_no_duplicates() {
        let mut file = ApprovalsFile::default();
        file.add_to_allowlist("bash", "/usr/bin/ls");
        file.add_to_allowlist("bash", "/usr/bin/ls");
        let config = file.tools.get("bash").unwrap();
        assert_eq!(config.allowlist.len(), 1);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");

        let mut original = ApprovalsFile::default();
        original.add_to_allowlist("bash", "/usr/bin/ls");
        original.add_to_allowlist("bash", "/usr/bin/cat");
        original.add_to_allowlist("editor", "/usr/bin/vim");
        original.save(&path).unwrap();

        let loaded = ApprovalsFile::load(&path).unwrap();
        assert_eq!(loaded.version, original.version);
        assert_eq!(loaded.tools.len(), 2);
        assert!(loaded.is_allowed("bash", "/usr/bin/ls"));
        assert!(loaded.is_allowed("bash", "/usr/bin/cat"));
        assert!(loaded.is_allowed("editor", "/usr/bin/vim"));
        assert!(!loaded.is_allowed("editor", "/usr/bin/emacs"));
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.json");
        let file = ApprovalsFile::load(&path).unwrap();
        assert_eq!(file.version, 1);
        assert!(file.tools.is_empty());
    }
}
