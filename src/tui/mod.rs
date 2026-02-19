// ABOUTME: TUI module — ratatui full-screen interface for soloclaw.
// ABOUTME: Chat display, input handling, status bar, and inline approval prompts.

pub mod input;
pub mod model;
pub mod state;
pub mod subscriptions;
pub mod ui;
pub mod widgets;

pub use state::*;
