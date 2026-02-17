// ABOUTME: Core types for the layered approval system.
// ABOUTME: SecurityLevel, AskMode, AskFallback, and ApprovalDecision enums.

use serde::{Deserialize, Serialize};

/// How restrictive the security policy is for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityLevel {
    /// Reject all invocations unconditionally.
    Deny,
    /// Allow only invocations matching the allowlist.
    Allowlist,
    /// Allow all invocations (subject to ask mode).
    Full,
}

/// When to prompt the user for approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AskMode {
    /// Never ask — auto-decide based on security level.
    Off,
    /// Ask only when the allowlist doesn't cover the invocation.
    OnMiss,
    /// Always ask, regardless of allowlist.
    Always,
}

/// What to do when an approval request times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskFallback {
    /// Deny on timeout.
    Deny,
    /// Fall back to allowlist check on timeout.
    Allowlist,
    /// Allow on timeout.
    Full,
}

/// The user's decision on an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Allow this one invocation.
    AllowOnce,
    /// Allow and add to persistent allowlist.
    AllowAlways,
    /// Deny this invocation.
    Deny,
}

/// The outcome of evaluating an approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Tool call is allowed without asking.
    Allow,
    /// Tool call is denied without asking.
    Denied,
    /// User must be prompted for approval.
    Ask,
}

/// Per-tool security configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSecurity {
    pub security: SecurityLevel,
    pub ask: AskMode,
    #[serde(default = "default_ask_fallback")]
    pub ask_fallback: AskFallback,
}

fn default_ask_fallback() -> AskFallback {
    AskFallback::Deny
}

impl Default for ToolSecurity {
    fn default() -> Self {
        Self {
            security: SecurityLevel::Allowlist,
            ask: AskMode::OnMiss,
            ask_fallback: AskFallback::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_level_serde_roundtrip() {
        let json = serde_json::to_string(&SecurityLevel::Allowlist).unwrap();
        assert_eq!(json, "\"allowlist\"");
        let parsed: SecurityLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SecurityLevel::Allowlist);
    }

    #[test]
    fn ask_mode_serde_roundtrip() {
        let json = serde_json::to_string(&AskMode::OnMiss).unwrap();
        assert_eq!(json, "\"on-miss\"");
        let parsed: AskMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AskMode::OnMiss);
    }

    #[test]
    fn tool_security_defaults() {
        let ts = ToolSecurity::default();
        assert_eq!(ts.security, SecurityLevel::Allowlist);
        assert_eq!(ts.ask, AskMode::OnMiss);
        assert_eq!(ts.ask_fallback, AskFallback::Deny);
    }

    #[test]
    fn tool_security_from_json() {
        let json = r#"{"security":"full","ask":"always"}"#;
        let ts: ToolSecurity = serde_json::from_str(json).unwrap();
        assert_eq!(ts.security, SecurityLevel::Full);
        assert_eq!(ts.ask, AskMode::Always);
        assert_eq!(ts.ask_fallback, AskFallback::Deny); // default
    }
}
