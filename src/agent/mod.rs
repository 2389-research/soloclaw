// ABOUTME: Agent module — LLM provider factory and streaming agent loop.
// ABOUTME: Manages conversation history and tool call dispatch.

pub mod r#loop;
pub mod provider;

pub use r#loop::run_agent_loop;
pub use provider::*;
