// ABOUTME: Session module — persistence of conversation history to disk.
// ABOUTME: Provides JSONL logging of messages per workspace session.

pub mod log;

pub use log::{SessionLogger, workspace_hash};
