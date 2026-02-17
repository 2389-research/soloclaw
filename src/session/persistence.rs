// ABOUTME: Session state persistence — save and load full conversation state as JSON.
// ABOUTME: Enables auto-resume of sessions per workspace directory via atomic file writes.

use std::path::{Path, PathBuf};

use chrono::Utc;
use mux::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::session::workspace_hash;

/// Full conversation state persisted between sessions.
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionState {
    pub workspace_dir: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<Message>,
    pub total_tokens: u64,
}

/// Path to the session state file for a given workspace directory.
pub fn session_state_path(workspace_dir: &Path) -> PathBuf {
    let hash = workspace_hash(workspace_dir);
    Config::sessions_dir().join(&hash).join("session.json")
}

/// Load a session state from disk, if it exists.
pub fn load_session(workspace_dir: &Path) -> anyhow::Result<Option<SessionState>> {
    let path = session_state_path(workspace_dir);
    load_session_from(&path)
}

/// Load a session state from an explicit file path (for testing).
pub fn load_session_from(path: &Path) -> anyhow::Result<Option<SessionState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    let state: SessionState = serde_json::from_str(&content)?;
    Ok(Some(state))
}

/// Save a session state to disk (atomic write via tmp + rename).
pub fn save_session(workspace_dir: &Path, state: &SessionState) -> anyhow::Result<()> {
    let path = session_state_path(workspace_dir);
    save_session_to(&path, state)
}

/// Save a session state to an explicit file path (for testing).
pub fn save_session_to(path: &Path, state: &SessionState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(&tmp_path, &content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Create a new SessionState for the given workspace and model.
pub fn new_session_state(workspace_dir: &Path, model: &str) -> SessionState {
    let now = Utc::now().to_rfc3339();
    SessionState {
        workspace_dir: workspace_dir.to_string_lossy().to_string(),
        model: model.to_string(),
        created_at: now.clone(),
        updated_at: now,
        messages: Vec::new(),
        total_tokens: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Helper: build a SessionState with some messages for testing.
    fn sample_session_state() -> SessionState {
        SessionState {
            workspace_dir: "/home/user/projects/myapp".to_string(),
            model: "claude-sonnet-4".to_string(),
            created_at: "2026-01-15T10:00:00+00:00".to_string(),
            updated_at: "2026-01-15T10:05:00+00:00".to_string(),
            messages: vec![
                Message::user("Hello, how are you?"),
                Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::text("I'm doing well, thanks!")],
                },
                Message::user("Can you list files?"),
                Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::ToolUse {
                        id: "call-1".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "ls"}),
                    }],
                },
                Message {
                    role: Role::User,
                    content: vec![ContentBlock::ToolResult {
                        tool_use_id: "call-1".to_string(),
                        content: "file1.txt\nfile2.txt".to_string(),
                        is_error: false,
                    }],
                },
            ],
            total_tokens: 1234,
        }
    }

    #[test]
    fn session_state_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("workspace_abc").join("session.json");

        let original = sample_session_state();
        save_session_to(&session_path, &original).unwrap();

        let loaded = load_session_from(&session_path).unwrap();
        assert!(loaded.is_some(), "should load a saved session");

        let loaded = loaded.unwrap();
        assert_eq!(loaded.workspace_dir, original.workspace_dir);
        assert_eq!(loaded.model, original.model);
        assert_eq!(loaded.created_at, original.created_at);
        assert_eq!(loaded.updated_at, original.updated_at);
        assert_eq!(loaded.total_tokens, original.total_tokens);
        assert_eq!(loaded.messages.len(), original.messages.len());

        // Verify first user message content.
        assert_eq!(loaded.messages[0].role, Role::User);
        match &loaded.messages[0].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello, how are you?"),
            other => panic!("expected Text, got {:?}", other),
        }

        // Verify assistant message with tool use.
        assert_eq!(loaded.messages[3].role, Role::Assistant);
        match &loaded.messages[3].content[0] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "call-1");
                assert_eq!(name, "bash");
                assert_eq!(input, &serde_json::json!({"command": "ls"}));
            }
            other => panic!("expected ToolUse, got {:?}", other),
        }

        // Verify tool result message.
        assert_eq!(loaded.messages[4].role, Role::User);
        match &loaded.messages[4].content[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "call-1");
                assert_eq!(content, "file1.txt\nfile2.txt");
                assert!(!is_error);
            }
            other => panic!("expected ToolResult, got {:?}", other),
        }
    }

    #[test]
    fn session_state_path_is_deterministic() {
        let path_a = Path::new("/home/user/projects/myapp");
        let result1 = session_state_path(path_a);
        let result2 = session_state_path(path_a);
        assert_eq!(result1, result2, "same workspace should produce same path");
    }

    #[test]
    fn load_nonexistent_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let missing_path = tmp.path().join("does_not_exist").join("session.json");
        let result = load_session_from(&missing_path).unwrap();
        assert!(result.is_none(), "loading from nonexistent path should return None");
    }

    #[test]
    fn save_is_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("workspace_xyz").join("session.json");

        let state = sample_session_state();
        save_session_to(&session_path, &state).unwrap();

        // The final file should exist.
        assert!(session_path.exists(), "session.json should exist after save");

        // The temporary file should NOT exist after a successful save.
        let tmp_path = session_path.with_extension("json.tmp");
        assert!(
            !tmp_path.exists(),
            "session.json.tmp should not exist after successful save"
        );
    }

    #[test]
    fn new_session_state_creates_empty() {
        let ws = Path::new("/tmp/test_workspace");
        let state = new_session_state(ws, "test-model");
        assert_eq!(state.workspace_dir, "/tmp/test_workspace");
        assert_eq!(state.model, "test-model");
        assert!(state.messages.is_empty());
        assert_eq!(state.total_tokens, 0);
        assert!(!state.created_at.is_empty());
        assert!(!state.updated_at.is_empty());
    }

    #[test]
    fn save_overwrites_existing_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session_path = tmp.path().join("workspace_overwrite").join("session.json");

        let mut state = sample_session_state();
        save_session_to(&session_path, &state).unwrap();

        // Modify and save again.
        state.messages.push(Message::user("Another message"));
        state.total_tokens = 9999;
        save_session_to(&session_path, &state).unwrap();

        let loaded = load_session_from(&session_path).unwrap().unwrap();
        assert_eq!(loaded.messages.len(), 6);
        assert_eq!(loaded.total_tokens, 9999);
    }
}
