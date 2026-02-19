// ABOUTME: Boba Model implementation — ClawApp is the Elm Architecture TUI.
// ABOUTME: All TUI state, message handling, and rendering lives here.

use std::sync::Arc;
use std::time::Instant;

use boba::widgets::text_area;
use boba::widgets::text_area::TextArea;
use boba::{subscribe, terminal_events, Command, Model, Subscription, TerminalEvent};
use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::Frame;
use tokio::sync::{mpsc, Mutex};

use crate::tui::state::{
    AgentEvent, ChatMessage, ChatMessageKind, PendingApproval, PendingQuestion, UserEvent,
};
use crate::tui::subscriptions::AgentEventSource;

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

    fn update(&mut self, _msg: Msg) -> Command<Msg> {
        Command::none() // Filled in by Tasks 4-7
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
}
