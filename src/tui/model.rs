// ABOUTME: Boba Model implementation — ClawApp is the Elm Architecture TUI.
// ABOUTME: All TUI state, message handling, and rendering lives here.

use std::sync::Arc;
use std::time::Instant;

use boba::widgets::text_area;
use boba::widgets::text_area::TextArea;
use boba::{subscribe, terminal_events, Command, Component, Model, Subscription, TerminalEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Frame;
use tokio::sync::{mpsc, Mutex};

use crate::tui::state::{
    AgentEvent, ChatMessage, ChatMessageKind, PendingApproval, PendingQuestion, ToolCallStatus,
    UserEvent,
};
use crate::tui::subscriptions::AgentEventSource;

const MOUSE_SCROLL_STEP: u16 = 3;

/// Messages that drive the ClawApp update cycle.
pub enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Agent(AgentEvent),
    Input(text_area::Message),
    MessageSent,
}

/// Initialization data passed to ClawApp::init.
pub struct Flags {
    pub user_tx: mpsc::Sender<UserEvent>,
    pub agent_rx: mpsc::Receiver<AgentEvent>,
    pub model_name: String,
    pub tool_count: usize,
    pub context_window: u64,
    pub workspace_dir: String,
    pub replay_messages: Vec<ChatMessage>,
    pub startup_message: String,
}

/// The top-level TUI application state, driven by the boba runtime.
pub struct ClawApp {
    pub input: TextArea,
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: u16,
    pub streaming: bool,
    pub queued_message: Option<String>,
    pub pending_approval: Option<PendingApproval>,
    pub pending_question: Option<PendingQuestion>,
    pub model_name: String,
    pub tool_count: usize,
    pub total_tokens: u64,
    pub context_window: u64,
    pub context_used: u64,
    pub session_start: Instant,
    pub workspace_dir: String,
    user_tx: mpsc::Sender<UserEvent>,
    agent_rx: Arc<Mutex<Option<mpsc::Receiver<AgentEvent>>>>,
}

impl Model for ClawApp {
    type Message = Msg;
    type Flags = Flags;

    fn init(flags: Flags) -> (Self, Command<Msg>) {
        let mut input = TextArea::new();
        input.focus(); // Start focused so typing works immediately

        let mut app = ClawApp {
            input,
            messages: Vec::new(),
            scroll_offset: 0,
            streaming: false,
            queued_message: None,
            pending_approval: None,
            pending_question: None,
            model_name: flags.model_name,
            tool_count: flags.tool_count,
            total_tokens: 0,
            context_window: flags.context_window,
            context_used: 0,
            session_start: Instant::now(),
            workspace_dir: flags.workspace_dir,
            user_tx: flags.user_tx,
            agent_rx: Arc::new(Mutex::new(Some(flags.agent_rx))),
        };

        if !flags.startup_message.is_empty() {
            app.push_message(ChatMessageKind::System, flags.startup_message);
        }

        for msg in flags.replay_messages {
            app.messages.push(msg);
        }
        if app.messages.len() > 1 {
            // more than just startup message
            app.push_message(
                ChatMessageKind::System,
                "\u{1f504} Session resumed".to_string(),
            );
        }

        (app, Command::none())
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::Agent(event) => match event {
                AgentEvent::TextDelta(text) => {
                    self.append_to_last_assistant(&text);
                    Command::none()
                }
                AgentEvent::TextDone => Command::none(),
                AgentEvent::ToolCallStarted {
                    tool_name,
                    params_summary,
                } => {
                    let content = format!("{}({})", tool_name, params_summary);
                    self.push_message(
                        ChatMessageKind::ToolCall {
                            tool_name,
                            status: ToolCallStatus::Pending,
                        },
                        content,
                    );
                    Command::none()
                }
                AgentEvent::ToolCallApproved { tool_name } => {
                    self.update_tool_status(&tool_name, ToolCallStatus::Allowed);
                    Command::none()
                }
                AgentEvent::ToolCallNeedsApproval {
                    description,
                    pattern,
                    tool_name,
                    responder,
                } => {
                    self.pending_approval = Some(PendingApproval {
                        description,
                        pattern,
                        tool_name,
                        selected: 0,
                        responder: Some(responder),
                    });
                    self.scroll_offset = 0;
                    Command::none()
                }
                AgentEvent::AskUser {
                    question,
                    tool_call_id,
                    options,
                    responder,
                } => {
                    self.pending_question = Some(PendingQuestion {
                        question,
                        tool_call_id,
                        options,
                        selected: 0,
                        responder: Some(responder),
                    });
                    self.scroll_offset = 0;
                    Command::none()
                }
                AgentEvent::ToolCallDenied { tool_name, reason } => {
                    self.update_tool_status(&tool_name, ToolCallStatus::Denied);
                    self.push_message(
                        ChatMessageKind::System,
                        format!("Tool '{}' denied: {}", tool_name, reason),
                    );
                    Command::none()
                }
                AgentEvent::ToolResult {
                    tool_name: _,
                    content,
                    is_error,
                } => {
                    self.push_message(ChatMessageKind::ToolResult { is_error }, content);
                    Command::none()
                }
                AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    self.total_tokens += (input_tokens + output_tokens) as u64;
                    self.context_used = input_tokens as u64;
                    Command::none()
                }
                AgentEvent::Error(msg) => {
                    self.push_message(
                        ChatMessageKind::System,
                        format!("\u{26a0}\u{fe0f} Error: {}", msg),
                    );
                    self.streaming = false;
                    Command::none()
                }
                AgentEvent::Done => {
                    self.streaming = false;
                    if let Some(queued) = self.queued_message.take() {
                        self.push_message(ChatMessageKind::User, queued.clone());
                        self.streaming = true;
                        return self.send_message(queued);
                    }
                    Command::none()
                }
                AgentEvent::CompactionStarted => {
                    self.push_message(
                        ChatMessageKind::System,
                        "\u{1f5dc}\u{fe0f} Compacting conversation...".to_string(),
                    );
                    Command::none()
                }
                AgentEvent::CompactionDone {
                    old_count,
                    new_count,
                } => {
                    self.push_message(
                        ChatMessageKind::System,
                        format!(
                            "\u{2705} Compacted: {} messages \u{2192} {} messages",
                            old_count, new_count
                        ),
                    );
                    Command::none()
                }
            },
            Msg::Key(key) => {
                // Ctrl+C always quits
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    return Command::quit();
                }

                // If pending approval or question, defer to Tasks 6-7
                if self.pending_approval.is_some() || self.pending_question.is_some() {
                    return Command::none();
                }

                match key.code {
                    KeyCode::PageUp => {
                        self.scroll_offset = self.scroll_offset.saturating_add(10);
                        Command::none()
                    }
                    KeyCode::PageDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(10);
                        Command::none()
                    }
                    KeyCode::Up if self.streaming => {
                        self.scroll_offset = self.scroll_offset.saturating_add(1);
                        Command::none()
                    }
                    KeyCode::Down if self.streaming => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(1);
                        Command::none()
                    }
                    KeyCode::Up => {
                        if self.input.cursor_row() == 0 {
                            self.scroll_offset = self.scroll_offset.saturating_add(1);
                            Command::none()
                        } else {
                            self.input
                                .update(text_area::Message::KeyPress(key))
                                .map(Msg::Input)
                        }
                    }
                    KeyCode::Down => {
                        if self.input.cursor_row()
                            >= self.input.line_count().saturating_sub(1)
                        {
                            self.scroll_offset = self.scroll_offset.saturating_sub(1);
                            Command::none()
                        } else {
                            self.input
                                .update(text_area::Message::KeyPress(key))
                                .map(Msg::Input)
                        }
                    }
                    KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                        let text = self.input.value();
                        if text.trim().is_empty() {
                            return Command::none();
                        }
                        if self.streaming {
                            self.queued_message = Some(text);
                            self.input.set_value("");
                            Command::none()
                        } else {
                            self.push_message(ChatMessageKind::User, text.clone());
                            self.streaming = true;
                            self.input.set_value("");
                            self.send_message(text)
                        }
                    }
                    KeyCode::Esc => {
                        if self.streaming {
                            Command::none()
                        } else {
                            Command::quit()
                        }
                    }
                    _ => self
                        .input
                        .update(text_area::Message::KeyPress(key))
                        .map(Msg::Input),
                }
            }
            Msg::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    self.scroll_offset =
                        self.scroll_offset.saturating_add(MOUSE_SCROLL_STEP);
                    Command::none()
                }
                MouseEventKind::ScrollDown => {
                    self.scroll_offset =
                        self.scroll_offset.saturating_sub(MOUSE_SCROLL_STEP);
                    Command::none()
                }
                _ => Command::none(),
            },
            Msg::Paste(text) => {
                if self.pending_approval.is_some() {
                    Command::none()
                } else {
                    self.input
                        .update(text_area::Message::Paste(text))
                        .map(Msg::Input)
                }
            }
            Msg::Input(_) => Command::none(),
            Msg::MessageSent => Command::none(),
        }
    }

    fn view(&self, _frame: &mut Frame) {
        // Filled in by Task 8
    }

    fn subscriptions(&self) -> Vec<Subscription<Msg>> {
        vec![
            terminal_events(|ev| match ev {
                TerminalEvent::Key(key) => Some(Msg::Key(key)),
                TerminalEvent::Mouse(mouse) => Some(Msg::Mouse(mouse)),
                TerminalEvent::Paste(text) => Some(Msg::Paste(text)),
                _ => None,
            }),
            subscribe(AgentEventSource {
                rx: self.agent_rx.clone(),
            })
            .map(Msg::Agent),
        ]
    }
}

impl ClawApp {
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

    /// Update the status of the most recent tool call message matching the given tool name.
    fn update_tool_status(&mut self, tool_name: &str, new_status: ToolCallStatus) {
        for msg in self.messages.iter_mut().rev() {
            if let ChatMessageKind::ToolCall {
                tool_name: ref name,
                ref mut status,
            } = msg.kind
            {
                if name == tool_name {
                    *status = new_status;
                    return;
                }
            }
        }
    }

    /// Send a user message to the agent loop via the mpsc channel.
    fn send_message(&self, text: String) -> Command<Msg> {
        let tx = self.user_tx.clone();
        Command::perform(
            async move {
                let _ = tx.send(UserEvent::Message(text)).await;
            },
            |_| Msg::MessageSent,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_flags() -> Flags {
        let (user_tx, _user_rx) = mpsc::channel(16);
        let (_agent_tx, agent_rx) = mpsc::channel(64);
        Flags {
            user_tx,
            agent_rx,
            model_name: "test-model".to_string(),
            tool_count: 5,
            context_window: 128_000,
            workspace_dir: "/tmp/test".to_string(),
            replay_messages: vec![],
            startup_message: "Test startup".to_string(),
        }
    }

    #[test]
    fn init_creates_valid_state() {
        let flags = test_flags();
        let (app, _cmd) = ClawApp::init(flags);

        assert_eq!(app.model_name, "test-model");
        assert_eq!(app.tool_count, 5);
        assert_eq!(app.context_window, 128_000);
        assert!(!app.streaming);
        assert!(app.pending_approval.is_none());
        assert!(app.pending_question.is_none());
        // Startup message should be present
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].kind, ChatMessageKind::System);
        assert_eq!(app.messages[0].content, "Test startup");
    }

    #[test]
    fn push_message_resets_scroll() {
        let flags = test_flags();
        let (mut app, _cmd) = ClawApp::init(flags);

        app.scroll_offset = 10;
        app.push_message(ChatMessageKind::User, "hello".to_string());
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn append_to_last_assistant() {
        let flags = test_flags();
        let (mut app, _cmd) = ClawApp::init(flags);

        app.push_message(ChatMessageKind::Assistant, "Hello".to_string());
        app.append_to_last_assistant(" world");
        // Should still be a single assistant message (plus the startup system message)
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.messages[1].content, "Hello world");
    }

    #[test]
    fn append_creates_new_if_no_assistant() {
        let flags = test_flags();
        let (mut app, _cmd) = ClawApp::init(flags);

        app.push_message(ChatMessageKind::User, "hi".to_string());
        app.append_to_last_assistant("response");
        // Should have: system startup + user msg + new assistant msg
        assert_eq!(app.messages.len(), 3);
        assert_eq!(app.messages[2].kind, ChatMessageKind::Assistant);
        assert_eq!(app.messages[2].content, "response");
    }

    #[test]
    fn init_with_replay_messages() {
        let (user_tx, _user_rx) = mpsc::channel(16);
        let (_agent_tx, agent_rx) = mpsc::channel(64);
        let flags = Flags {
            user_tx,
            agent_rx,
            model_name: "test-model".to_string(),
            tool_count: 5,
            context_window: 128_000,
            workspace_dir: "/tmp/test".to_string(),
            replay_messages: vec![
                ChatMessage {
                    kind: ChatMessageKind::User,
                    content: "replayed user msg".to_string(),
                },
                ChatMessage {
                    kind: ChatMessageKind::Assistant,
                    content: "replayed assistant msg".to_string(),
                },
            ],
            startup_message: "Test startup".to_string(),
        };

        let (app, _cmd) = ClawApp::init(flags);

        // Should have: startup message + 2 replay messages + "Session resumed"
        assert_eq!(app.messages.len(), 4);
        assert_eq!(app.messages[0].kind, ChatMessageKind::System);
        assert_eq!(app.messages[0].content, "Test startup");
        assert_eq!(app.messages[1].kind, ChatMessageKind::User);
        assert_eq!(app.messages[1].content, "replayed user msg");
        assert_eq!(app.messages[2].kind, ChatMessageKind::Assistant);
        assert_eq!(app.messages[2].content, "replayed assistant msg");
        assert_eq!(app.messages[3].kind, ChatMessageKind::System);
        assert!(app.messages[3].content.contains("Session resumed"));
    }

    // --- Agent event update() tests ---

    #[test]
    fn update_text_delta_appends() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::TextDelta("Hello".to_string())));
        app.update(Msg::Agent(AgentEvent::TextDelta(" world".to_string())));

        // Startup message + one assistant message
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.messages[1].kind, ChatMessageKind::Assistant);
        assert_eq!(app.messages[1].content, "Hello world");
    }

    #[test]
    fn update_done_stops_streaming() {
        let (mut app, _cmd) = ClawApp::init(test_flags());
        app.streaming = true;

        let cmd = app.update(Msg::Agent(AgentEvent::Done));

        assert!(!app.streaming);
        assert!(cmd.is_none());
    }

    #[test]
    fn update_done_sends_queued_message() {
        let (mut app, _cmd) = ClawApp::init(test_flags());
        app.streaming = true;
        app.queued_message = Some("follow up".to_string());

        let cmd = app.update(Msg::Agent(AgentEvent::Done));

        assert!(app.streaming); // re-set to true for the queued send
        assert!(app.queued_message.is_none());
        assert!(!cmd.is_none()); // should have returned a send command
        // The queued message should have been pushed as a User message
        let user_msgs: Vec<_> = app
            .messages
            .iter()
            .filter(|m| m.kind == ChatMessageKind::User)
            .collect();
        assert_eq!(user_msgs.len(), 1);
        assert_eq!(user_msgs[0].content, "follow up");
    }

    #[test]
    fn update_error_stops_streaming() {
        let (mut app, _cmd) = ClawApp::init(test_flags());
        app.streaming = true;

        app.update(Msg::Agent(AgentEvent::Error("oops".to_string())));

        assert!(!app.streaming);
        let last = app.messages.last().unwrap();
        assert_eq!(last.kind, ChatMessageKind::System);
        assert!(last.content.contains("oops"));
    }

    #[test]
    fn update_tool_call_started() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::ToolCallStarted {
            tool_name: "read_file".to_string(),
            params_summary: "path=/tmp".to_string(),
        }));

        let last = app.messages.last().unwrap();
        assert_eq!(
            last.kind,
            ChatMessageKind::ToolCall {
                tool_name: "read_file".to_string(),
                status: ToolCallStatus::Pending,
            }
        );
        assert_eq!(last.content, "read_file(path=/tmp)");
    }

    #[test]
    fn update_tool_approved_updates_status() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::ToolCallStarted {
            tool_name: "write_file".to_string(),
            params_summary: "path=/tmp".to_string(),
        }));
        app.update(Msg::Agent(AgentEvent::ToolCallApproved {
            tool_name: "write_file".to_string(),
        }));

        let last = app.messages.last().unwrap();
        assert_eq!(
            last.kind,
            ChatMessageKind::ToolCall {
                tool_name: "write_file".to_string(),
                status: ToolCallStatus::Allowed,
            }
        );
    }

    #[test]
    fn update_needs_approval_sets_pending() {
        let (mut app, _cmd) = ClawApp::init(test_flags());
        app.scroll_offset = 5;

        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.update(Msg::Agent(AgentEvent::ToolCallNeedsApproval {
            description: "Write to disk".to_string(),
            pattern: Some("write_*".to_string()),
            tool_name: "write_file".to_string(),
            responder: tx,
        }));

        assert!(app.pending_approval.is_some());
        let approval = app.pending_approval.as_ref().unwrap();
        assert_eq!(approval.description, "Write to disk");
        assert_eq!(approval.tool_name, "write_file");
        assert_eq!(approval.pattern, Some("write_*".to_string()));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn update_ask_user_sets_pending_question() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.update(Msg::Agent(AgentEvent::AskUser {
            question: "What is your name?".to_string(),
            tool_call_id: "call-42".to_string(),
            options: vec!["Alice".to_string(), "Bob".to_string()],
            responder: tx,
        }));

        assert!(app.pending_question.is_some());
        let q = app.pending_question.as_ref().unwrap();
        assert_eq!(q.question, "What is your name?");
        assert_eq!(q.tool_call_id, "call-42");
        assert_eq!(q.options, vec!["Alice", "Bob"]);
    }

    #[test]
    fn update_usage_tracks_tokens() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::Usage {
            input_tokens: 100,
            output_tokens: 50,
        }));

        assert_eq!(app.total_tokens, 150);
        assert_eq!(app.context_used, 100);
    }

    #[test]
    fn update_compaction_messages() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::CompactionStarted));
        let compacting_msg = app.messages.last().unwrap();
        assert_eq!(compacting_msg.kind, ChatMessageKind::System);
        assert!(compacting_msg.content.contains("Compacting"));

        app.update(Msg::Agent(AgentEvent::CompactionDone {
            old_count: 50,
            new_count: 10,
        }));
        let done_msg = app.messages.last().unwrap();
        assert_eq!(done_msg.kind, ChatMessageKind::System);
        assert!(done_msg.content.contains("50"));
        assert!(done_msg.content.contains("10"));
        assert!(done_msg.content.contains("Compacted"));
    }

    // --- Key, Mouse, Paste handling tests (Task 5) ---

    #[test]
    fn key_esc_quits() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(!cmd.is_none());
    }

    #[test]
    fn key_esc_during_streaming_does_nothing() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
    }

    #[test]
    fn key_enter_sends_message() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.input.set_value("hello world");
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(!cmd.is_none());
        assert!(app.streaming);
        assert_eq!(app.input.value(), "");
        // User message should have been pushed
        let user_msgs: Vec<_> = app
            .messages
            .iter()
            .filter(|m| m.kind == ChatMessageKind::User)
            .collect();
        assert_eq!(user_msgs.len(), 1);
        assert_eq!(user_msgs[0].content, "hello world");
    }

    #[test]
    fn key_enter_empty_does_nothing() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
        assert!(!app.streaming);
    }

    #[test]
    fn key_enter_during_streaming_queues() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        app.input.set_value("follow up");
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.queued_message, Some("follow up".to_string()));
        assert_eq!(app.input.value(), "");
    }

    #[test]
    fn key_ctrl_c_quits() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let cmd = app.update(Msg::Key(key));
        assert!(!cmd.is_none());
    }

    #[test]
    fn key_pageup_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.scroll_offset, 10);
    }

    #[test]
    fn key_pagedown_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.scroll_offset = 15;
        let key = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.scroll_offset, 5);
    }

    #[test]
    fn mouse_scroll_up() {
        let (mut app, _) = ClawApp::init(test_flags());
        let mouse = crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        app.update(Msg::Mouse(mouse));
        assert_eq!(app.scroll_offset, MOUSE_SCROLL_STEP);
    }

    #[test]
    fn mouse_scroll_down() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.scroll_offset = 10;
        let mouse = crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        app.update(Msg::Mouse(mouse));
        assert_eq!(app.scroll_offset, 10 - MOUSE_SCROLL_STEP);
    }

    #[test]
    fn paste_inserts_text() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.update(Msg::Paste("pasted text".to_string()));
        assert!(app.input.value().contains("pasted text"));
    }

    #[test]
    fn paste_blocked_during_approval() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        app.update(Msg::Paste("should not appear".to_string()));
        assert!(!app.input.value().contains("should not appear"));
    }

    #[test]
    fn key_up_on_first_line_scrolls_chat() {
        let (mut app, _) = ClawApp::init(test_flags());
        // Input has a single line, cursor is on row 0
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn key_up_during_streaming_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn key_down_during_streaming_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        app.scroll_offset = 5;
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.scroll_offset, 4);
    }

    #[test]
    fn msg_input_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let cmd = app.update(Msg::Input(text_area::Message::KeyPress(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        ))));
        assert!(cmd.is_none());
    }

    #[test]
    fn msg_message_sent_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let cmd = app.update(Msg::MessageSent);
        assert!(cmd.is_none());
    }

    #[test]
    fn key_during_pending_approval_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
    }

    #[test]
    fn key_during_pending_question_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "test?".to_string(),
            tool_call_id: "call-1".to_string(),
            options: vec![],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
    }

    #[test]
    fn update_tool_denied() {
        let (mut app, _cmd) = ClawApp::init(test_flags());

        app.update(Msg::Agent(AgentEvent::ToolCallStarted {
            tool_name: "rm_rf".to_string(),
            params_summary: "path=/".to_string(),
        }));
        app.update(Msg::Agent(AgentEvent::ToolCallDenied {
            tool_name: "rm_rf".to_string(),
            reason: "too dangerous".to_string(),
        }));

        // The tool call message should now have Denied status
        let tool_msg = app
            .messages
            .iter()
            .find(|m| {
                matches!(
                    &m.kind,
                    ChatMessageKind::ToolCall { tool_name, .. } if tool_name == "rm_rf"
                )
            })
            .unwrap();
        assert_eq!(
            tool_msg.kind,
            ChatMessageKind::ToolCall {
                tool_name: "rm_rf".to_string(),
                status: ToolCallStatus::Denied,
            }
        );

        // A system message about the denial should have been pushed
        let denial_msg = app.messages.last().unwrap();
        assert_eq!(denial_msg.kind, ChatMessageKind::System);
        assert!(denial_msg.content.contains("rm_rf"));
        assert!(denial_msg.content.contains("denied"));
        assert!(denial_msg.content.contains("too dangerous"));
    }
}
