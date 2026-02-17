// ABOUTME: JSONL session logger — appends each conversation message to a log file.
// ABOUTME: Stores logs per workspace in ~/.local/share/soloclaw/sessions/<workspace_hash>/.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use mux::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;

/// A single JSONL log entry containing a timestamp and the conversation message.
#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub message: Message,
}

/// Computes a deterministic hex hash of the workspace directory path.
pub fn workspace_hash(workspace_dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    workspace_dir.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Appends conversation messages as JSONL lines to a session log file.
pub struct SessionLogger {
    writer: BufWriter<File>,
    pub session_dir: PathBuf,
}

impl SessionLogger {
    /// Create a session logger for the given workspace directory.
    ///
    /// Creates the session directory structure and opens a new JSONL log file
    /// named with the current ISO timestamp.
    pub fn new(workspace_dir: &Path) -> anyhow::Result<Self> {
        let hash = workspace_hash(workspace_dir);
        let session_dir = Config::sessions_dir().join(&hash);
        Self::create_in_dir(&session_dir)
    }

    /// Create a session logger that writes to a specific directory (for testing).
    pub fn new_in_dir(session_dir: &Path) -> anyhow::Result<Self> {
        Self::create_in_dir(session_dir)
    }

    /// Shared constructor: creates the directory and opens a timestamped JSONL file.
    fn create_in_dir(session_dir: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(session_dir)?;
        let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%S").to_string();
        let log_path = session_dir.join(format!("{}.jsonl", timestamp));
        let file = File::create(&log_path)?;
        let writer = BufWriter::new(file);
        Ok(Self {
            writer,
            session_dir: session_dir.to_path_buf(),
        })
    }

    /// Append a message to the JSONL log file.
    pub fn log_message(&mut self, msg: &Message) -> anyhow::Result<()> {
        let entry = LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            message: msg.clone(),
        };
        let line = serde_json::to_string(&entry)?;
        writeln!(self.writer, "{}", line)?;
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn workspace_hash_is_deterministic() {
        let path = Path::new("/home/user/projects/myapp");
        let hash1 = workspace_hash(path);
        let hash2 = workspace_hash(path);
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16, "hash should be 16 hex characters");
    }

    #[test]
    fn workspace_hash_differs_for_different_paths() {
        let path_a = Path::new("/home/user/project-a");
        let path_b = Path::new("/home/user/project-b");
        let hash_a = workspace_hash(path_a);
        let hash_b = workspace_hash(path_b);
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn session_logger_writes_valid_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let session_dir = tmp.path().join("sessions").join("test_workspace");

        let mut logger = SessionLogger::new_in_dir(&session_dir).unwrap();
        let msg = Message::user("Hello, world!");
        logger.log_message(&msg).unwrap();

        // Read back the JSONL file.
        let entries: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .collect();
        assert_eq!(entries.len(), 1, "should have exactly one JSONL file");

        let content = fs::read_to_string(entries[0].path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1, "should have exactly one line");

        // Each line should be valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert!(parsed.get("timestamp").is_some(), "should have timestamp field");
        assert!(parsed.get("message").is_some(), "should have message field");
    }

    #[test]
    fn session_logger_message_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let session_dir = tmp.path().join("sessions").join("roundtrip");

        let mut logger = SessionLogger::new_in_dir(&session_dir).unwrap();
        let msg = Message::user("test content for roundtrip");
        logger.log_message(&msg).unwrap();

        // Read back and deserialize.
        let entries: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .collect();
        let content = fs::read_to_string(entries[0].path()).unwrap();
        let entry: LogEntry = serde_json::from_str(content.lines().next().unwrap()).unwrap();

        assert_eq!(entry.message.role, Role::User);
        // The content should contain our text.
        let text = match &entry.message.content[0] {
            ContentBlock::Text { text } => text.clone(),
            other => panic!("expected Text block, got {:?}", other),
        };
        assert_eq!(text, "test content for roundtrip");
    }

    #[test]
    fn session_logger_multiple_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let session_dir = tmp.path().join("sessions").join("multi");

        let mut logger = SessionLogger::new_in_dir(&session_dir).unwrap();
        logger.log_message(&Message::user("first")).unwrap();
        logger.log_message(&Message::user("second")).unwrap();
        logger
            .log_message(&Message {
                role: Role::Assistant,
                content: vec![ContentBlock::text("response")],
            })
            .unwrap();

        let entries: Vec<_> = fs::read_dir(&session_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
            .collect();
        let content = fs::read_to_string(entries[0].path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3, "should have three lines");

        // All lines should parse as valid LogEntry.
        for line in &lines {
            let _entry: LogEntry = serde_json::from_str(line).unwrap();
        }
    }
}
