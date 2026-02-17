// ABOUTME: Integration tests for the approval engine.
// ABOUTME: Tests the full flow: policy + allowlist + analysis + persistence.

use std::collections::HashMap;

use soloclaw::approval::{
    ApprovalDecision, ApprovalEngine, ApprovalsFile, AskMode, EngineOutcome, SecurityLevel,
    ToolApprovalConfig, ToolCallInfo, ToolSecurity,
};

/// Full flow: bash with a safe command (grep) should be auto-allowed
/// through the entire engine pipeline — analysis detects the safe bin,
/// policy evaluates it, and the engine returns Allowed.
#[test]
fn full_approval_flow_bash_safe_command() {
    let approvals = ApprovalsFile::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.json");
    let engine = ApprovalEngine::with_approvals(approvals, path);

    let info = ToolCallInfo {
        tool_name: "bash".to_string(),
        params: serde_json::json!({ "command": "grep -r 'TODO' src/" }),
    };

    let outcome = engine.check(&info);
    assert_eq!(outcome, EngineOutcome::Allowed);
}

/// Full flow: bash with an unsafe command (cargo build) initially returns
/// NeedsApproval. After resolving with AllowAlways the pattern is persisted,
/// and re-checking the same command returns Allowed.
#[test]
fn full_approval_flow_bash_unsafe_then_allow_always() {
    let approvals = ApprovalsFile::default();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.json");

    // Save initial approvals so the engine can persist changes.
    approvals.save(&path).unwrap();

    let engine = ApprovalEngine::with_approvals(approvals, path.clone());

    let info = ToolCallInfo {
        tool_name: "bash".to_string(),
        params: serde_json::json!({ "command": "cargo build" }),
    };

    // First check: cargo is not a safe bin, so it should require approval.
    let outcome = engine.check(&info);
    let pattern = match &outcome {
        EngineOutcome::NeedsApproval { pattern, .. } => pattern.clone(),
        other => panic!("expected NeedsApproval, got {:?}", other),
    };

    // Resolve with AllowAlways using the extracted pattern.
    engine.resolve("bash", pattern.as_deref(), ApprovalDecision::AllowAlways);

    // Second check: the pattern is now in the allowlist, so it should be allowed.
    let outcome_after = engine.check(&info);
    assert_eq!(outcome_after, EngineOutcome::Allowed);

    // Verify persistence: reload from disk and confirm the pattern is there.
    let reloaded = ApprovalsFile::load(&path).unwrap();
    if let Some(pat) = &pattern {
        assert!(
            reloaded.is_allowed("bash", pat),
            "pattern {:?} should be in the persisted allowlist",
            pat,
        );
    }
}

/// When a wildcard "*" tool entry has SecurityLevel::Deny, every tool call
/// should be denied regardless of tool name or parameters. This tests the
/// full chain: wildcard lookup → policy evaluation → Denied outcome.
#[test]
fn deny_security_blocks_everything() {
    let mut tools = HashMap::new();
    tools.insert(
        "*".to_string(),
        ToolApprovalConfig {
            security: ToolSecurity {
                security: SecurityLevel::Deny,
                ask: AskMode::Off,
                ..ToolSecurity::default()
            },
            allowlist: Vec::new(),
        },
    );
    let approvals = ApprovalsFile {
        version: 1,
        defaults: ToolSecurity::default(),
        tools,
    };

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.json");
    let engine = ApprovalEngine::with_approvals(approvals, path);

    let info = ToolCallInfo {
        tool_name: "read_file".to_string(),
        params: serde_json::json!({ "path": "/etc/hosts" }),
    };

    let outcome = engine.check(&info);
    match outcome {
        EngineOutcome::Denied { .. } => {} // expected
        other => panic!("expected Denied, got {:?}", other),
    }

    // Also verify bash tools are denied under the wildcard.
    let bash_info = ToolCallInfo {
        tool_name: "bash".to_string(),
        params: serde_json::json!({ "command": "ls" }),
    };
    let bash_outcome = engine.check(&bash_info);
    match bash_outcome {
        EngineOutcome::Denied { .. } => {} // expected
        other => panic!("expected Denied for bash, got {:?}", other),
    }
}
