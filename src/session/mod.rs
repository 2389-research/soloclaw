// ABOUTME: Session module — persistence of conversation history to disk.
// ABOUTME: Provides JSONL logging of messages per workspace session.

pub mod log;
pub mod persistence;

pub use log::{SessionLogger, workspace_hash};
pub use persistence::{SessionState, load_session, save_session, new_session_state};
