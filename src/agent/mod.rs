// ABOUTME: Agent module — LLM provider factory and streaming agent loop.
// ABOUTME: Manages conversation history and tool call dispatch.

pub mod compaction;
pub mod r#loop;
pub mod provider;

pub use r#loop::{AgentLoopParams, run_agent_loop};
pub use provider::*;
