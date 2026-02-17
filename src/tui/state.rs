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
    /// The LLM is asking the user a question via the ask_user tool.
    AskUser {
        question: String,
        tool_call_id: String,
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
    /// One-shot channel to send the user's answer back to the agent loop.
    pub responder: Option<oneshot::Sender<String>>,
}

/// Full TUI application state.
pub struct TuiState {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor_pos: usize,
    pub scroll_offset: u16,
    pub streaming: bool,
    pub pending_approval: Option<PendingApproval>,
    pub pending_question: Option<PendingQuestion>,
    pub model: String,
    pub tool_count: usize,
    pub total_tokens: u64,
    pub context_window: u64,
    pub context_used: u64,
    pub session_start: std::time::Instant,
    pub workspace_dir: String,
    pub queued_message: Option<String>,
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
            pending_question: None,
            model,
            tool_count,
            total_tokens: 0,
            context_window: 128_000,
            context_used: 0,
            session_start: std::time::Instant::now(),
            workspace_dir: String::new(),
            queued_message: None,
        }
    }

    /// Add a message to the chat history and reset scroll to bottom.
    pub fn push_message(&mut self, kind: ChatMessageKind, content: String) {
        self.messages.push(ChatMessage { kind, content });
        self.scroll_offset = 0;
    }

    /// Append text to the last assistant message, or create a new one if needed.
    /// Keeps scroll pinned to the bottom so new content is always visible.
    pub fn append_to_last_assistant(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut() {
            if msg.kind == ChatMessageKind::Assistant {
                msg.content.push_str(text);
                self.scroll_offset = 0;
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

    /// Clamp the cursor position to the valid character range of the input buffer.
    pub fn clamp_cursor(&mut self) {
        self.cursor_pos = self.cursor_pos.min(self.input_char_len());
    }

    /// Return the current cursor byte index in the UTF-8 input buffer.
    pub fn cursor_byte_index(&self) -> usize {
        char_index_to_byte_index(&self.input, self.cursor_pos)
    }

    /// Return the total number of characters in the input buffer.
    pub fn input_char_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Insert a character at the cursor and advance by one character.
    pub fn insert_char_at_cursor(&mut self, c: char) {
        self.clamp_cursor();
        let byte_index = self.cursor_byte_index();
        self.input.insert(byte_index, c);
        self.cursor_pos += 1;
    }

    /// Insert a string at the current cursor position.
    pub fn insert_str_at_cursor(&mut self, s: &str) {
        self.clamp_cursor();
        let byte_index = self.cursor_byte_index();
        self.input.insert_str(byte_index, s);
        self.cursor_pos += s.chars().count();
    }

    /// Delete the character before the cursor (backspace behavior).
    pub fn backspace_char(&mut self) {
        self.clamp_cursor();
        if self.cursor_pos == 0 {
            return;
        }

        let end = self.cursor_byte_index();
        let start = char_index_to_byte_index(&self.input, self.cursor_pos - 1);
        self.input.replace_range(start..end, "");
        self.cursor_pos -= 1;
    }

    /// Delete the character at the cursor (delete behavior).
    pub fn delete_char_at_cursor(&mut self) {
        self.clamp_cursor();
        if self.cursor_pos >= self.input_char_len() {
            return;
        }

        let start = self.cursor_byte_index();
        let end = char_index_to_byte_index(&self.input, self.cursor_pos + 1);
        self.input.replace_range(start..end, "");
    }

    /// Move cursor one character to the left.
    pub fn move_cursor_left(&mut self) {
        self.clamp_cursor();
        self.cursor_pos = self.cursor_pos.saturating_sub(1);
    }

    /// Move cursor one character to the right.
    pub fn move_cursor_right(&mut self) {
        self.clamp_cursor();
        if self.cursor_pos < self.input_char_len() {
            self.cursor_pos += 1;
        }
    }

    /// Move cursor to start of input.
    pub fn move_cursor_home(&mut self) {
        self.cursor_pos = 0;
    }

    /// Move cursor to end of input.
    pub fn move_cursor_end(&mut self) {
        self.cursor_pos = self.input_char_len();
    }

    /// Whether there is a pending approval prompt.
    pub fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }

    /// Whether there is a pending question from the LLM.
    pub fn has_pending_question(&self) -> bool {
        self.pending_question.is_some()
    }

    /// Split the input on newlines.
    pub fn input_lines(&self) -> Vec<&str> {
        self.input.split('\n').collect()
    }

    /// Which line the cursor is currently on (0-indexed).
    pub fn cursor_line(&self) -> usize {
        let byte_idx = self.cursor_byte_index();
        self.input[..byte_idx].matches('\n').count()
    }

    /// Column position (in characters) within the current line.
    pub fn cursor_column(&self) -> usize {
        let byte_idx = self.cursor_byte_index();
        let text_before = &self.input[..byte_idx];
        match text_before.rfind('\n') {
            Some(nl_pos) => text_before[nl_pos + 1..].chars().count(),
            None => text_before.chars().count(),
        }
    }

    /// Number of lines in the input buffer.
    pub fn input_line_count(&self) -> usize {
        self.input.split('\n').count()
    }

    /// Move cursor up one line within the input. Returns false if already at line 0.
    pub fn move_cursor_up_in_input(&mut self) -> bool {
        let line = self.cursor_line();
        if line == 0 {
            return false;
        }
        let col = self.cursor_column();
        let lines = self.input_lines();
        let target_col = col.min(lines[line - 1].chars().count());
        // Calculate new cursor_pos (char-based)
        let mut pos = 0;
        for (i, l) in lines.iter().enumerate() {
            if i == line - 1 {
                pos += target_col;
                break;
            }
            pos += l.chars().count() + 1; // +1 for \n
        }
        self.cursor_pos = pos;
        true
    }

    /// Move cursor down one line within the input. Returns false if already at last line.
    pub fn move_cursor_down_in_input(&mut self) -> bool {
        let line = self.cursor_line();
        let lines = self.input_lines();
        if line >= lines.len() - 1 {
            return false;
        }
        let col = self.cursor_column();
        let target_col = col.min(lines[line + 1].chars().count());
        let mut pos = 0;
        for (i, l) in lines.iter().enumerate() {
            if i == line + 1 {
                pos += target_col;
                break;
            }
            pos += l.chars().count() + 1; // +1 for \n
        }
        self.cursor_pos = pos;
        true
    }
}

fn char_index_to_byte_index(s: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }

    match s.char_indices().nth(char_index) {
        Some((idx, _)) => idx,
        None => s.len(),
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
        assert!(!state.has_pending_question());
        assert_eq!(state.model, "claude-sonnet");
        assert_eq!(state.tool_count, 5);
        assert_eq!(state.total_tokens, 0);
    }

    #[test]
    fn pending_question_lifecycle() {
        let mut state = TuiState::new("m".to_string(), 0);
        assert!(!state.has_pending_question());

        let (tx, _rx) = oneshot::channel();
        state.pending_question = Some(PendingQuestion {
            question: "What is your name?".to_string(),
            tool_call_id: "call-42".to_string(),
            responder: Some(tx),
        });
        assert!(state.has_pending_question());

        let q = state.pending_question.as_ref().unwrap();
        assert_eq!(q.question, "What is your name?");
        assert_eq!(q.tool_call_id, "call-42");

        state.pending_question = None;
        assert!(!state.has_pending_question());
    }

    #[test]
    fn pending_question_responder_sends() {
        let (tx, rx) = oneshot::channel();
        let question = PendingQuestion {
            question: "test?".to_string(),
            tool_call_id: "id-1".to_string(),
            responder: Some(tx),
        };
        question.responder.unwrap().send("my answer".to_string()).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), "my answer");
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

    #[test]
    fn utf8_input_editing_is_safe() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.insert_char_at_cursor('a');
        state.insert_char_at_cursor('🙂');
        state.insert_char_at_cursor('é');
        assert_eq!(state.input, "a🙂é");
        assert_eq!(state.cursor_pos, 3);

        state.move_cursor_left();
        state.backspace_char();
        assert_eq!(state.input, "aé");
        assert_eq!(state.cursor_pos, 1);

        state.delete_char_at_cursor();
        assert_eq!(state.input, "a");
        assert_eq!(state.cursor_pos, 1);
    }

    #[test]
    fn clamp_cursor_handles_out_of_range_positions() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hi🙂".to_string();
        state.cursor_pos = 999;
        state.clamp_cursor();
        assert_eq!(state.cursor_pos, 3);
        assert_eq!(state.cursor_byte_index(), state.input.len());
    }

    #[test]
    fn insert_str_at_beginning() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.insert_str_at_cursor("hello");
        assert_eq!(state.input, "hello");
        assert_eq!(state.cursor_pos, 5);
    }

    #[test]
    fn insert_str_at_middle() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "ac".to_string();
        state.cursor_pos = 1;
        state.insert_str_at_cursor("b");
        assert_eq!(state.input, "abc");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn insert_str_at_end() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 5;
        state.insert_str_at_cursor(" world");
        assert_eq!(state.input, "hello world");
        assert_eq!(state.cursor_pos, 11);
    }

    #[test]
    fn insert_str_with_unicode() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "aé".to_string();
        state.cursor_pos = 1;
        state.insert_str_at_cursor("🙂");
        assert_eq!(state.input, "a🙂é");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn insert_str_empty_string() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 3;
        state.insert_str_at_cursor("");
        assert_eq!(state.input, "hello");
        assert_eq!(state.cursor_pos, 3);
    }

    // --- Multiline input helper tests ---

    #[test]
    fn cursor_line_single_line() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 3;
        assert_eq!(state.cursor_line(), 0);
    }

    #[test]
    fn cursor_line_multiline() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef\nghi".to_string();
        // cursor_pos 5 means 5 chars in: 'a','b','c','\n','d' => on 'd', which is line 1
        state.cursor_pos = 5;
        assert_eq!(state.cursor_line(), 1);
    }

    #[test]
    fn cursor_column_first_line() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        state.cursor_pos = 2;
        assert_eq!(state.cursor_column(), 2);
    }

    #[test]
    fn cursor_column_second_line() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        // cursor_pos 5 means 5 chars: 'a','b','c','\n','d' => col 1 on line 1
        state.cursor_pos = 5;
        assert_eq!(state.cursor_column(), 1);
    }

    #[test]
    fn input_lines_splits() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "a\nb\nc".to_string();
        assert_eq!(state.input_lines(), vec!["a", "b", "c"]);
    }

    #[test]
    fn input_line_count_multiline() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "a\nb\nc".to_string();
        assert_eq!(state.input_line_count(), 3);
    }

    #[test]
    fn move_up_at_first_line_returns_false() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        state.cursor_pos = 2; // on line 0
        assert!(!state.move_cursor_up_in_input());
    }

    #[test]
    fn move_up_moves_cursor() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        // cursor_pos 5: 'a','b','c','\n','d' => col 1 on line 1
        state.cursor_pos = 5;
        assert!(state.move_cursor_up_in_input());
        // Should move to col 1 on line 0 => cursor_pos 1
        assert_eq!(state.cursor_pos, 1);
    }

    #[test]
    fn move_down_at_last_line_returns_false() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        state.cursor_pos = 5; // on last line
        assert!(!state.move_cursor_down_in_input());
    }

    #[test]
    fn move_down_moves_cursor() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        // cursor_pos 1: on 'b', col 1 on line 0
        state.cursor_pos = 1;
        assert!(state.move_cursor_down_in_input());
        // Should move to col 1 on line 1 => chars: 'a','b','c','\n','d' => pos 5
        assert_eq!(state.cursor_pos, 5);
    }
}
