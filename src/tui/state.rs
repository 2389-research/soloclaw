// ABOUTME: TUI state types — chat messages, agent/user events, input buffer, and approval state.
// ABOUTME: Drives the TUI rendering and bridges the agent loop to the display.

use tokio::sync::oneshot;

use crate::approval::ApprovalDecision;

/// The kind of a single chat message displayed in the TUI.
#[derive(Debug, PartialEq)]
pub enum ChatMessageKind {
    User,
    Assistant,
    ToolCall {
        tool_name: String,
        status: ToolCallStatus,
    },
    ToolResult {
        is_error: bool,
    },
    System,
}

/// Status of a tool call as it progresses through approval.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStatus {
    Allowed,
    Denied,
    Pending,
    TimedOut,
}

/// A single message in the chat history.
#[derive(Debug)]
pub struct ChatMessage {
    pub kind: ChatMessageKind,
    pub content: String,
}

/// Events sent from the agent loop to the TUI via an mpsc channel.
pub enum AgentEvent {
    /// Streaming text delta from the LLM.
    TextDelta(String),
    /// Streaming text is complete.
    TextDone,
    /// A tool call has started execution.
    ToolCallStarted {
        tool_name: String,
        params_summary: String,
    },
    /// A tool call was approved (auto or by user).
    ToolCallApproved { tool_name: String },
    /// A tool call needs user approval via the TUI.
    ToolCallNeedsApproval {
        description: String,
        pattern: Option<String>,
        tool_name: String,
        responder: oneshot::Sender<ApprovalDecision>,
    },
    /// A tool call was denied.
    ToolCallDenied {
        tool_name: String,
        reason: String,
    },
    /// A tool call completed with a result.
    ToolResult {
        tool_name: String,
        content: String,
        is_error: bool,
    },
    /// An error occurred in the agent loop.
    Error(String),
    /// The agent loop finished processing.
    Done,
}

/// Events sent from the TUI to the agent loop.
pub enum UserEvent {
    /// User submitted a chat message.
    Message(String),
    /// User requested to quit.
    Quit,
}

/// A pending approval prompt shown inline in the TUI.
pub struct PendingApproval {
    pub description: String,
    pub pattern: Option<String>,
    pub tool_name: String,
    /// Index of the currently selected option (0=AllowOnce, 1=AllowAlways, 2=Deny).
    pub selected: usize,
    /// One-shot channel to send the user's decision back to the agent loop.
    pub responder: Option<oneshot::Sender<ApprovalDecision>>,
}

/// Full TUI application state.
pub struct TuiState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor_pos: usize,
    pub scroll_offset: u16,
    pub streaming: bool,
    pub pending_approval: Option<PendingApproval>,
    pub model: String,
    pub tool_count: usize,
    pub total_tokens: u64,
}

impl TuiState {
    /// Create a new empty TUI state with the given model name and tool count.
    pub fn new(model: String, tool_count: usize) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            streaming: false,
            pending_approval: None,
            model,
            tool_count,
            total_tokens: 0,
        }
    }

    /// Add a message to the chat history and reset scroll to bottom.
    pub fn push_message(&mut self, kind: ChatMessageKind, content: String) {
        self.messages.push(ChatMessage { kind, content });
        self.scroll_offset = 0;
    }

    /// Append text to the last assistant message, or create a new one if needed.
    pub fn append_to_last_assistant(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut() {
            if msg.kind == ChatMessageKind::Assistant {
                msg.content.push_str(text);
                return;
            }
        }
        self.push_message(ChatMessageKind::Assistant, text.to_string());
    }

    /// Submit the current input buffer. Returns the trimmed text if non-empty.
    pub fn submit_input(&mut self) -> Option<String> {
        let trimmed = self.input.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        self.input.clear();
        self.cursor_pos = 0;
        Some(trimmed)
    }

    /// Whether there is a pending approval prompt.
    pub fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_empty() {
        let state = TuiState::new("claude-sonnet".to_string(), 5);
        assert!(state.messages.is_empty());
        assert_eq!(state.input, "");
        assert_eq!(state.cursor_pos, 0);
        assert_eq!(state.scroll_offset, 0);
        assert!(!state.streaming);
        assert!(!state.has_pending_approval());
        assert_eq!(state.model, "claude-sonnet");
        assert_eq!(state.tool_count, 5);
        assert_eq!(state.total_tokens, 0);
    }

    #[test]
    fn push_message_auto_scrolls() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.scroll_offset = 10;
        state.push_message(ChatMessageKind::User, "hello".to_string());
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "hello");
    }

    #[test]
    fn append_to_streaming_message() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.push_message(ChatMessageKind::Assistant, "Hello".to_string());
        state.append_to_last_assistant(" world");
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "Hello world");
    }

    #[test]
    fn append_creates_new_if_no_assistant() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.push_message(ChatMessageKind::User, "hi".to_string());
        state.append_to_last_assistant("response");
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].kind, ChatMessageKind::Assistant);
        assert_eq!(state.messages[1].content, "response");
    }

    #[test]
    fn submit_input_clears_buffer() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "  hello world  ".to_string();
        state.cursor_pos = 10;
        let result = state.submit_input();
        assert_eq!(result, Some("hello world".to_string()));
        assert_eq!(state.input, "");
        assert_eq!(state.cursor_pos, 0);
    }

    #[test]
    fn submit_empty_input_returns_none() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "   ".to_string();
        let result = state.submit_input();
        assert_eq!(result, None);
        // Input is NOT cleared when empty
        assert_eq!(state.input, "   ");
    }
}
