// ABOUTME: Approval engine — orchestrates policy, allowlist, and command analysis.
// ABOUTME: Evaluates tool calls against security config and persists allow-always decisions.

use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use super::{
    allowlist::ApprovalsFile,
    analysis::{allowlist_pattern, analyze_command},
    policy::evaluate_approval,
    types::{ApprovalDecision, ApprovalOutcome},
};

/// Information about a tool call to be evaluated by the engine.
pub struct ToolCallInfo {
    pub tool_name: String,
    pub params: Value,
}

/// The outcome of the engine's evaluation of a tool call.
#[derive(Debug, PartialEq, Eq)]
pub enum EngineOutcome {
    /// The tool call is allowed to proceed.
    Allowed,
    /// The tool call is denied.
    Denied { reason: String },
    /// The tool call requires user approval before proceeding.
    NeedsApproval {
        description: String,
        pattern: Option<String>,
    },
}

/// Orchestrator that ties together policy, allowlist, and command analysis
/// to decide whether a tool call should be allowed, denied, or require approval.
pub struct ApprovalEngine {
    approvals: Mutex<ApprovalsFile>,
    approvals_path: PathBuf,
}

impl ApprovalEngine {
    /// Create a new engine by loading an ApprovalsFile from disk.
    pub fn new(approvals_path: PathBuf) -> anyhow::Result<Self> {
        let approvals = ApprovalsFile::load(&approvals_path)?;
        Ok(Self {
            approvals: Mutex::new(approvals),
            approvals_path,
        })
    }

    /// Create an engine from an existing ApprovalsFile, useful for testing.
    pub fn with_approvals(approvals: ApprovalsFile, path: PathBuf) -> Self {
        Self {
            approvals: Mutex::new(approvals),
            approvals_path: path,
        }
    }

    /// Evaluate a tool call and return the engine's decision.
    ///
    /// For "bash" tools, performs command analysis (safe-bin detection, allowlist matching).
    /// For other tools, checks whether the tool name appears in its own allowlist.
    pub fn check(&self, info: &ToolCallInfo) -> EngineOutcome {
        let approvals = self.approvals.lock().expect("approvals lock poisoned");
        let tool_sec = approvals.tool_security(&info.tool_name);
        let security = tool_sec.security;
        let ask = tool_sec.ask;

        if info.tool_name == "bash" {
            let (allowlist_satisfied, pattern) = self.check_bash(&approvals, &info.params);

            let outcome = evaluate_approval(security, ask, allowlist_satisfied);
            match outcome {
                ApprovalOutcome::Allow => EngineOutcome::Allowed,
                ApprovalOutcome::Denied => EngineOutcome::Denied {
                    reason: "denied by policy".to_string(),
                },
                ApprovalOutcome::Ask => EngineOutcome::NeedsApproval {
                    description: self.describe_tool_call(info),
                    pattern,
                },
            }
        } else {
            // For non-bash tools, check if the tool name itself is in the allowlist.
            let allowlist_satisfied = approvals.is_allowed(&info.tool_name, &info.tool_name);

            let outcome = evaluate_approval(security, ask, allowlist_satisfied);
            match outcome {
                ApprovalOutcome::Allow => EngineOutcome::Allowed,
                ApprovalOutcome::Denied => EngineOutcome::Denied {
                    reason: "denied by policy".to_string(),
                },
                ApprovalOutcome::Ask => EngineOutcome::NeedsApproval {
                    description: self.describe_tool_call(info),
                    pattern: Some(info.tool_name.clone()),
                },
            }
        }
    }

    /// Resolve a pending approval by recording the user's decision.
    ///
    /// If the decision is AllowAlways, the pattern is added to the allowlist and persisted.
    pub fn resolve(&self, tool_name: &str, pattern: Option<&str>, decision: ApprovalDecision) {
        if decision == ApprovalDecision::AllowAlways {
            if let Some(pat) = pattern {
                let mut approvals = self.approvals.lock().expect("approvals lock poisoned");
                approvals.add_to_allowlist(tool_name, pat);
                // Best-effort save — callers should handle errors if critical.
                let _ = approvals.save(&self.approvals_path);
            }
        }
    }

    /// Extract the command from bash params, analyze it, and check safe-bin/allowlist status.
    ///
    /// Returns (allowlist_satisfied, pattern) where pattern is the resolved executable path
    /// or executable name for potential allowlisting.
    fn check_bash(&self, approvals: &ApprovalsFile, params: &Value) -> (bool, Option<String>) {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let analysis = analyze_command(command);

        // Safe commands (all segments use safe bins) are auto-approved.
        if analysis.safe {
            return (true, None);
        }

        // Check if the resolved executable is in the allowlist.
        let pattern = allowlist_pattern(&analysis);
        let allowlist_satisfied = pattern
            .as_ref()
            .map(|p| approvals.is_allowed("bash", p))
            .unwrap_or(false);

        (allowlist_satisfied, pattern)
    }

    /// Format a tool call for display, truncating params to 60 characters.
    fn describe_tool_call(&self, info: &ToolCallInfo) -> String {
        let params_str = info.params.to_string();
        let truncated = if params_str.len() > 60 {
            format!("{}...", &params_str[..60])
        } else {
            params_str
        };
        format!("{}({})", info.tool_name, truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::allowlist::ToolApprovalConfig;
    use crate::approval::types::{AskMode, SecurityLevel, ToolSecurity};
    use std::collections::HashMap;

    /// Build an ApprovalsFile with bash (Allowlist+OnMiss) and read_file (Full+Off).
    fn test_approvals() -> ApprovalsFile {
        let mut tools = HashMap::new();
        tools.insert(
            "bash".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Allowlist,
                    ask: AskMode::OnMiss,
                    ..ToolSecurity::default()
                },
                allowlist: Vec::new(),
            },
        );
        tools.insert(
            "read_file".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Full,
                    ask: AskMode::Off,
                    ..ToolSecurity::default()
                },
                allowlist: Vec::new(),
            },
        );
        ApprovalsFile {
            version: 1,
            defaults: ToolSecurity::default(),
            tools,
        }
    }

    #[test]
    fn bash_safe_command_auto_approves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(test_approvals(), path);

        let info = ToolCallInfo {
            tool_name: "bash".to_string(),
            params: serde_json::json!({ "command": "cat file.txt | grep error" }),
        };

        assert_eq!(engine.check(&info), EngineOutcome::Allowed);
    }

    #[test]
    fn bash_unsafe_command_asks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(test_approvals(), path);

        let info = ToolCallInfo {
            tool_name: "bash".to_string(),
            params: serde_json::json!({ "command": "rm -rf /tmp/data" }),
        };

        let outcome = engine.check(&info);
        match outcome {
            EngineOutcome::NeedsApproval { .. } => {} // expected
            other => panic!("expected NeedsApproval, got {:?}", other),
        }
    }

    #[test]
    fn read_file_auto_approves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(test_approvals(), path);

        let info = ToolCallInfo {
            tool_name: "read_file".to_string(),
            params: serde_json::json!({ "path": "/etc/hosts" }),
        };

        // read_file has Full security + Off ask mode → auto-allow.
        assert_eq!(engine.check(&info), EngineOutcome::Allowed);
    }

    #[test]
    fn unknown_tool_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(test_approvals(), path);

        let info = ToolCallInfo {
            tool_name: "some_unknown_tool".to_string(),
            params: serde_json::json!({}),
        };

        // Defaults are Allowlist+OnMiss, not in allowlist → NeedsApproval.
        let outcome = engine.check(&info);
        match outcome {
            EngineOutcome::NeedsApproval { .. } => {} // expected
            other => panic!("expected NeedsApproval, got {:?}", other),
        }
    }

    #[test]
    fn resolve_allow_always_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");

        // Save initial approvals to disk so we can verify changes.
        let approvals = test_approvals();
        approvals.save(&path).unwrap();

        let engine = ApprovalEngine::with_approvals(approvals, path.clone());

        engine.resolve("bash", Some("/usr/bin/rm"), ApprovalDecision::AllowAlways);

        // Verify the pattern was persisted to disk.
        let reloaded = ApprovalsFile::load(&path).unwrap();
        assert!(reloaded.is_allowed("bash", "/usr/bin/rm"));
    }

    #[test]
    fn resolve_allow_once_does_not_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");

        let approvals = test_approvals();
        approvals.save(&path).unwrap();

        let engine = ApprovalEngine::with_approvals(approvals, path.clone());

        engine.resolve("bash", Some("/usr/bin/rm"), ApprovalDecision::AllowOnce);

        // Verify the pattern was NOT persisted.
        let reloaded = ApprovalsFile::load(&path).unwrap();
        assert!(!reloaded.is_allowed("bash", "/usr/bin/rm"));
    }
}
