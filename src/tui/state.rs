// ABOUTME: TUI shared types — chat messages, agent/user events, and approval/question state.
// ABOUTME: Bridges the agent loop to the boba-based TUI display layer.

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
    /// The LLM is asking the user a question via the ask_user tool.
    AskUser {
        question: String,
        tool_call_id: String,
        /// Multiple-choice options. Empty means free-text mode.
        options: Vec<String>,
        responder: oneshot::Sender<String>,
    },
    /// A tool call was denied.
    ToolCallDenied { tool_name: String, reason: String },
    /// A tool call completed with a result.
    ToolResult {
        tool_name: String,
        content: String,
        is_error: bool,
    },
    /// Token usage update from a completed API response.
    Usage {
        input_tokens: u32,
        output_tokens: u32,
    },
    /// An error occurred in the agent loop.
    Error(String),
    /// The agent loop finished processing.
    Done,
    /// Compaction has started.
    CompactionStarted,
    /// Compaction is complete.
    CompactionDone { old_count: usize, new_count: usize },
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

/// A pending question from the LLM shown inline in the TUI.
pub struct PendingQuestion {
    pub question: String,
    pub tool_call_id: String,
    /// Multiple-choice options. Empty means free-text mode.
    pub options: Vec<String>,
    /// Index of the currently selected option (for multiple choice).
    pub selected: usize,
    /// One-shot channel to send the user's answer back to the agent loop.
    pub responder: Option<oneshot::Sender<String>>,
}

