# Boba TUI Migration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Migrate soloclaw's TUI from raw ratatui + manual event loop to the boba framework (Elm Architecture).

**Architecture:** Replace the manual 50ms poll loop in app.rs with boba's Model trait. Agent events arrive via a custom SubscriptionSource wrapping the mpsc channel. TextArea replaces our hand-rolled multiline input. All backend code (agent loop, approval engine, session, compaction) stays untouched.

**Tech Stack:** boba (path dep at `../boba`), ratatui (via boba), crossterm (for key event types), tokio-stream (for ReceiverStream)

**Design Doc:** `docs/plans/2026-02-19-boba-migration-design.md`

---

### Task 1: Project Setup

**Files:**
- Modify: `Cargo.toml`

**Step 1: Create feature branch**

```bash
git checkout -b feat/boba-migration
```

**Step 2: Add boba dependency**

Add to `[dependencies]` in `Cargo.toml`:
```toml
boba = { path = "../boba" }
tokio-stream = "0.1"
```

**Step 3: Verify build**

Run: `cargo build`
Expected: Compiles successfully. Boba resolves from sibling path.

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add boba and tokio-stream dependencies"
```

---

### Task 2: Create AgentEventSource Subscription

**Files:**
- Create: `src/tui/subscriptions.rs`
- Modify: `src/tui/mod.rs`

**Step 1: Write the failing test**

Create `src/tui/subscriptions.rs`:

```rust
// ABOUTME: Custom boba SubscriptionSource for agent loop events.
// ABOUTME: Wraps the mpsc::Receiver<AgentEvent> so boba's runtime manages it.

use std::sync::Arc;

use boba::{SubscriptionId, SubscriptionSource};
use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;

use crate::tui::state::AgentEvent;

/// Subscription source that streams AgentEvent from the agent loop's mpsc channel.
/// The receiver is stored in Arc<Mutex<Option<>>> because subscriptions() is called
/// every update cycle, but the stream is only consumed once (when the subscription
/// first starts). Subsequent cycles see the same SubscriptionId and keep it alive.
pub struct AgentEventSource {
    pub rx: Arc<Mutex<Option<mpsc::Receiver<AgentEvent>>>>,
}

impl SubscriptionSource for AgentEventSource {
    type Output = AgentEvent;

    fn id(&self) -> SubscriptionId {
        SubscriptionId::of::<Self>()
    }

    fn stream(self) -> BoxStream<'static, AgentEvent> {
        Box::pin(futures::stream::unfold(self.rx, |rx_slot| async move {
            let mut guard = rx_slot.lock().await;
            let rx = guard.take()?;
            drop(guard);
            let mut stream = ReceiverStream::new(rx);
            let first = stream.next().await?;
            // Return first item, then continue streaming via a chained stream.
            // We need to yield first, then yield the rest.
            Some((first, rx_slot)) // This only yields one item per unfold step.
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_event_source_has_stable_id() {
        let (_tx, rx) = mpsc::channel::<AgentEvent>(1);
        let source1 = AgentEventSource {
            rx: Arc::new(Mutex::new(Some(rx))),
        };
        let (_tx2, rx2) = mpsc::channel::<AgentEvent>(1);
        let source2 = AgentEventSource {
            rx: Arc::new(Mutex::new(Some(rx2))),
        };
        assert_eq!(source1.id(), source2.id());
    }
}
```

Wait — the `unfold` approach only yields one item per step and needs rethinking. Better approach: use `async_stream` or a simpler pattern. Revise the stream() implementation:

```rust
fn stream(self) -> BoxStream<'static, AgentEvent> {
    Box::pin(async_stream::stream! {
        let mut guard = self.rx.lock().await;
        if let Some(rx) = guard.take() {
            drop(guard);
            let mut stream = ReceiverStream::new(rx);
            while let Some(event) = stream.next().await {
                yield event;
            }
        }
    })
}
```

If `async_stream` is not desired as a dependency, use this alternative without it:

```rust
fn stream(self) -> BoxStream<'static, AgentEvent> {
    Box::pin(futures::stream::unfold(
        Some(self.rx),
        |state| async move {
            match state {
                Some(rx_slot) => {
                    let rx = rx_slot.lock().await.take()?;
                    // Convert to stream and yield all items
                    let stream = ReceiverStream::new(rx);
                    // Wrap remaining items
                    Some(()) // Need different approach
                }
                None => None,
            }
        },
    ))
}
```

**Simplest correct approach**: Take the receiver in stream(), convert directly to a stream:

```rust
fn stream(self) -> BoxStream<'static, AgentEvent> {
    let rx_slot = self.rx;
    Box::pin(futures::stream::once(async move {
        rx_slot.lock().await.take()
    })
    .filter_map(|opt| async { opt })
    .map(ReceiverStream::new)
    .flatten())
}
```

**Step 2: Add module to tui/mod.rs**

Add `pub mod subscriptions;` to `src/tui/mod.rs`.

**Step 3: Run test to verify**

Run: `cargo test tui::subscriptions`
Expected: PASS

**Step 4: Commit**

```bash
git add src/tui/subscriptions.rs src/tui/mod.rs
git commit -m "feat: add AgentEventSource subscription for boba"
```

---

### Task 3: Create ClawApp Model Skeleton

**Files:**
- Create: `src/tui/model.rs`
- Modify: `src/tui/mod.rs`

**Step 1: Write the ClawApp struct, Msg enum, Flags struct**

Create `src/tui/model.rs` with the following structure:

```rust
// ABOUTME: Boba Model implementation — ClawApp is the Elm Architecture TUI.
// ABOUTME: All TUI state, message handling, and rendering lives here.

use std::sync::Arc;
use std::time::Instant;

use boba::{Command, Model, Subscription, subscribe, terminal_events, TerminalEvent};
use boba::widgets::TextArea;
use boba::widgets::text_area;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::{mpsc, Mutex};
use unicode_width::UnicodeWidthStr;

use crate::approval::ApprovalDecision;
use crate::tui::state::{
    AgentEvent, ChatMessage, ChatMessageKind, PendingApproval, PendingQuestion,
    ToolCallStatus, UserEvent,
};
use crate::tui::subscriptions::AgentEventSource;
use crate::tui::widgets::approval::approval_line;
use crate::tui::widgets::chat::render_chat_lines;
use crate::tui::widgets::question::{multichoice_lines, question_lines};
use crate::tui::widgets::status::{StatusBarParams, status_line};

const MOUSE_SCROLL_STEP: u16 = 3;
const MAX_INPUT_HEIGHT: u16 = 8;

pub enum Msg {
    Key(KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Paste(String),
    Agent(AgentEvent),
    Input(text_area::Message),
    MessageSent,
}

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
```

**Step 2: Implement minimal Model trait**

```rust
impl Model for ClawApp {
    type Message = Msg;
    type Flags = Flags;

    fn init(flags: Flags) -> (Self, Command<Msg>) {
        let mut app = ClawApp {
            input: TextArea::new(),
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

        // Add startup message.
        if !flags.startup_message.is_empty() {
            app.push_message(ChatMessageKind::System, flags.startup_message);
        }

        // Replay session messages.
        for msg in flags.replay_messages {
            app.messages.push(msg);
        }
        if !app.messages.is_empty() {
            app.push_message(ChatMessageKind::System, "🔄 Session resumed".to_string());
        }

        (app, Command::none())
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        Command::none() // Filled in by Tasks 4-7
    }

    fn view(&self, frame: &mut Frame) {
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
```

**Step 3: Add helper methods on ClawApp**

Port these from current TuiState — same logic, different struct:

```rust
impl ClawApp {
    pub fn push_message(&mut self, kind: ChatMessageKind, content: String) {
        self.messages.push(ChatMessage { kind, content });
        self.scroll_offset = 0;
    }

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
```

**Step 4: Write tests**

```rust
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
        let (app, _cmd) = ClawApp::init(test_flags());
        assert_eq!(app.model_name, "test-model");
        assert_eq!(app.tool_count, 5);
        assert_eq!(app.context_window, 128_000);
        assert!(!app.streaming);
        assert!(app.pending_approval.is_none());
        assert!(app.pending_question.is_none());
        assert_eq!(app.messages.len(), 1); // startup message
    }

    #[test]
    fn push_message_resets_scroll() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.scroll_offset = 10;
        app.push_message(ChatMessageKind::User, "hello".to_string());
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn append_to_last_assistant() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.push_message(ChatMessageKind::Assistant, "Hello".to_string());
        app.append_to_last_assistant(" world");
        let last = app.messages.last().unwrap();
        assert_eq!(last.content, "Hello world");
    }
}
```

**Step 5: Add module to tui/mod.rs**

Add `pub mod model;` to `src/tui/mod.rs`.

**Step 6: Run tests**

Run: `cargo test tui::model`
Expected: PASS

**Step 7: Commit**

```bash
git add src/tui/model.rs src/tui/mod.rs
git commit -m "feat: add ClawApp model skeleton with init and helpers"
```

---

### Task 4: Implement update() for Agent Events

**Files:**
- Modify: `src/tui/model.rs`

**Step 1: Write failing tests for agent event handling**

Add to the tests module in `model.rs`:

```rust
#[test]
fn update_text_delta_appends() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.update(Msg::Agent(AgentEvent::TextDelta("Hello".to_string())));
    assert_eq!(app.messages.last().unwrap().content, "Hello");
    app.update(Msg::Agent(AgentEvent::TextDelta(" world".to_string())));
    assert_eq!(app.messages.last().unwrap().content, "Hello world");
}

#[test]
fn update_done_stops_streaming() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.streaming = true;
    app.update(Msg::Agent(AgentEvent::Done));
    assert!(!app.streaming);
}

#[test]
fn update_done_sends_queued_message() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.streaming = true;
    app.queued_message = Some("follow up".to_string());
    let cmd = app.update(Msg::Agent(AgentEvent::Done));
    assert!(!app.streaming);
    assert!(app.queued_message.is_none());
    // cmd should contain a send_message command (non-none)
    assert!(!cmd.is_none());
}

#[test]
fn update_error_stops_streaming() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.streaming = true;
    app.update(Msg::Agent(AgentEvent::Error("oops".to_string())));
    assert!(!app.streaming);
    assert!(app.messages.last().unwrap().content.contains("oops"));
}

#[test]
fn update_tool_call_started() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.update(Msg::Agent(AgentEvent::ToolCallStarted {
        tool_name: "bash".to_string(),
        params_summary: r#"{"cmd":"ls"}"#.to_string(),
    }));
    let last = app.messages.last().unwrap();
    match &last.kind {
        ChatMessageKind::ToolCall { tool_name, status } => {
            assert_eq!(tool_name, "bash");
            assert_eq!(*status, ToolCallStatus::Pending);
        }
        _ => panic!("expected ToolCall"),
    }
}

#[test]
fn update_tool_approved_updates_status() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.update(Msg::Agent(AgentEvent::ToolCallStarted {
        tool_name: "bash".to_string(),
        params_summary: "{}".to_string(),
    }));
    app.update(Msg::Agent(AgentEvent::ToolCallApproved {
        tool_name: "bash".to_string(),
    }));
    let last_tool = app.messages.iter().rev().find(|m| {
        matches!(m.kind, ChatMessageKind::ToolCall { .. })
    }).unwrap();
    match &last_tool.kind {
        ChatMessageKind::ToolCall { status, .. } => {
            assert_eq!(*status, ToolCallStatus::Allowed);
        }
        _ => unreachable!(),
    }
}

#[test]
fn update_needs_approval_sets_pending() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.update(Msg::Agent(AgentEvent::ToolCallNeedsApproval {
        description: "bash(rm -rf /)".to_string(),
        pattern: Some("/usr/bin/rm".to_string()),
        tool_name: "bash".to_string(),
        responder: tx,
    }));
    assert!(app.pending_approval.is_some());
    assert_eq!(app.scroll_offset, 0);
}

#[test]
fn update_ask_user_sets_pending_question() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.update(Msg::Agent(AgentEvent::AskUser {
        question: "What?".to_string(),
        tool_call_id: "c1".to_string(),
        options: vec![],
        responder: tx,
    }));
    assert!(app.pending_question.is_some());
}

#[test]
fn update_usage_tracks_tokens() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.update(Msg::Agent(AgentEvent::Usage {
        input_tokens: 100,
        output_tokens: 50,
    }));
    assert_eq!(app.total_tokens, 150);
    assert_eq!(app.context_used, 100);
}

#[test]
fn update_compaction_messages() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.update(Msg::Agent(AgentEvent::CompactionStarted));
    assert!(app.messages.last().unwrap().content.contains("Compacting"));
    app.update(Msg::Agent(AgentEvent::CompactionDone {
        old_count: 50,
        new_count: 5,
    }));
    assert!(app.messages.last().unwrap().content.contains("50"));
}
```

**Step 2: Run tests — expect failures**

Run: `cargo test tui::model::tests`
Expected: FAIL (update returns Command::none for everything)

**Step 3: Implement agent event handling in update()**

Replace the `update()` body with a match on `Msg::Agent(event)`. Port the logic directly from the current `handle_agent_event()` function in `src/app.rs:447-555`. The mapping is 1:1 except:
- Return `Command::none()` instead of `LoopAction::Continue`
- For `Done` with queued message: take the queued text, call `self.push_message()` for it, set `self.streaming = true`, and return `self.send_message(text)`
- Add a `update_tool_status()` helper method on ClawApp (same logic as current free function in app.rs:558-571)

All other Msg variants return `Command::none()` for now (filled in by later tasks).

**Step 4: Run tests**

Run: `cargo test tui::model::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/tui/model.rs
git commit -m "feat: implement ClawApp update() for agent events"
```

---

### Task 5: Implement update() for Normal Key Events

**Files:**
- Modify: `src/tui/model.rs`

**Step 1: Write failing tests**

```rust
#[test]
fn key_esc_quits() {
    let (mut app, _) = ClawApp::init(test_flags());
    let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    let cmd = app.update(Msg::Key(key));
    // cmd should be Command::quit()
    // Check by inspecting command (or just verify it's not none)
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
    assert!(!cmd.is_none()); // should send message
    assert!(app.streaming); // should set streaming
    // Input should be cleared
    assert_eq!(app.input.value(), "");
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
fn paste_inserts_text() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.input.focus();
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
```

**Step 2: Run tests — expect failures**

Run: `cargo test tui::model::tests`
Expected: FAIL

**Step 3: Implement key handling in update()**

Add match arms for `Msg::Key(key)`, `Msg::Mouse(mouse)`, and `Msg::Paste(text)`.

Key routing logic (port from current `src/tui/input.rs` and `src/app.rs`):

```rust
Msg::Key(key) => {
    // Ctrl+C always quits
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Command::quit();
    }

    // Route to approval/question handlers if pending (Tasks 6-7)
    if self.pending_approval.is_some() {
        return self.handle_approval_key(key);
    }
    if self.pending_question.is_some() {
        return self.handle_question_key(key);
    }

    match key.code {
        // Enter without shift: send or queue
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            let text = self.input.value().trim().to_string();
            if text.is_empty() {
                return Command::none();
            }
            self.input.set_value("");
            if self.streaming {
                self.queued_message = Some(text);
                Command::none()
            } else {
                self.push_message(ChatMessageKind::User, text.clone());
                self.streaming = true;
                self.send_message(text)
            }
        }
        // Esc: quit (only when not streaming)
        KeyCode::Esc if !self.streaming => Command::quit(),
        // PageUp/PageDown: always scroll chat
        KeyCode::PageUp => {
            self.scroll_offset = self.scroll_offset.saturating_add(10);
            Command::none()
        }
        KeyCode::PageDown => {
            self.scroll_offset = self.scroll_offset.saturating_sub(10);
            Command::none()
        }
        // Up: move cursor in input if possible, else scroll
        KeyCode::Up if !self.streaming => {
            if self.input.cursor_row() == 0 {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                Command::none()
            } else {
                self.input.update(text_area::Message::KeyPress(key)).map(Msg::Input)
            }
        }
        // Down: move cursor in input if possible, else scroll
        KeyCode::Down if !self.streaming => {
            if self.input.cursor_row() >= self.input.line_count().saturating_sub(1) {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                Command::none()
            } else {
                self.input.update(text_area::Message::KeyPress(key)).map(Msg::Input)
            }
        }
        // During streaming: Up/Down always scroll
        KeyCode::Up if self.streaming => {
            self.scroll_offset = self.scroll_offset.saturating_add(1);
            Command::none()
        }
        KeyCode::Down if self.streaming => {
            self.scroll_offset = self.scroll_offset.saturating_sub(1);
            Command::none()
        }
        // Everything else: forward to TextArea (Shift+Enter becomes newline, typing, etc.)
        _ => self.input.update(text_area::Message::KeyPress(key)).map(Msg::Input),
    }
}
Msg::Mouse(mouse) => {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            self.scroll_offset = self.scroll_offset.saturating_add(MOUSE_SCROLL_STEP);
        }
        MouseEventKind::ScrollDown => {
            self.scroll_offset = self.scroll_offset.saturating_sub(MOUSE_SCROLL_STEP);
        }
        _ => {}
    }
    Command::none()
}
Msg::Paste(text) => {
    if self.pending_approval.is_none() {
        self.input.update(text_area::Message::Paste(text)).map(Msg::Input)
    } else {
        Command::none()
    }
}
Msg::Input(_) => Command::none(), // TextArea internal messages, no further action needed
Msg::MessageSent => Command::none(),
```

**Step 4: Run tests**

Run: `cargo test tui::model::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/tui/model.rs
git commit -m "feat: implement ClawApp key, mouse, and paste handling"
```

---

### Task 6: Implement update() for Approval Mode

**Files:**
- Modify: `src/tui/model.rs`

**Step 1: Write failing tests**

```rust
#[test]
fn approval_enter_sends_decision() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_approval = Some(PendingApproval {
        description: "bash(ls)".to_string(),
        pattern: None,
        tool_name: "bash".to_string(),
        selected: 0, // AllowOnce
        responder: Some(tx),
    });
    let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert!(app.pending_approval.is_none());
    assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::AllowOnce);
}

#[test]
fn approval_number_key_selects_and_sends() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_approval = Some(PendingApproval {
        description: "test".to_string(),
        pattern: None,
        tool_name: "bash".to_string(),
        selected: 0,
        responder: Some(tx),
    });
    // Press '2' for AllowAlways
    let key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert!(app.pending_approval.is_none());
    assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::AllowAlways);
}

#[test]
fn approval_right_arrow_navigates() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_approval = Some(PendingApproval {
        description: "test".to_string(),
        pattern: None,
        tool_name: "bash".to_string(),
        selected: 0,
        responder: Some(tx),
    });
    let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert_eq!(app.pending_approval.as_ref().unwrap().selected, 1);
}
```

**Step 2: Run tests — expect failures**

**Step 3: Implement handle_approval_key()**

Port logic from current `src/tui/input.rs` `handle_approval_key()`:

```rust
impl ClawApp {
    fn handle_approval_key(&mut self, key: KeyEvent) -> Command<Msg> {
        match key.code {
            KeyCode::Left => {
                if let Some(ref mut a) = self.pending_approval {
                    a.selected = a.selected.saturating_sub(1);
                }
                Command::none()
            }
            KeyCode::Right => {
                if let Some(ref mut a) = self.pending_approval {
                    a.selected = (a.selected + 1).min(2);
                }
                Command::none()
            }
            KeyCode::Char('1') => self.resolve_approval(0),
            KeyCode::Char('2') => self.resolve_approval(1),
            KeyCode::Char('3') => self.resolve_approval(2),
            KeyCode::Enter => {
                let selected = self.pending_approval.as_ref().map(|a| a.selected).unwrap_or(0);
                self.resolve_approval(selected)
            }
            _ => Command::none(),
        }
    }

    fn resolve_approval(&mut self, selected: usize) -> Command<Msg> {
        if let Some(mut approval) = self.pending_approval.take() {
            let decision = match selected {
                0 => ApprovalDecision::AllowOnce,
                1 => ApprovalDecision::AllowAlways,
                _ => ApprovalDecision::Deny,
            };
            if let Some(responder) = approval.responder.take() {
                let _ = responder.send(decision);
            }
        }
        Command::none()
    }
}
```

**Step 4: Run tests**

Run: `cargo test tui::model::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/tui/model.rs
git commit -m "feat: implement approval mode key handling in ClawApp"
```

---

### Task 7: Implement update() for Question Mode

**Files:**
- Modify: `src/tui/model.rs`

**Step 1: Write failing tests**

```rust
#[test]
fn question_freetext_enter_sends_answer() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_question = Some(PendingQuestion {
        question: "Name?".to_string(),
        tool_call_id: "c1".to_string(),
        options: vec![],
        selected: 0,
        responder: Some(tx),
    });
    app.input.set_value("Alice");
    let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert!(app.pending_question.is_none());
    assert_eq!(rx.blocking_recv().unwrap(), "Alice");
    assert_eq!(app.input.value(), ""); // input cleared
}

#[test]
fn question_esc_dismisses() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_question = Some(PendingQuestion {
        question: "Name?".to_string(),
        tool_call_id: "c1".to_string(),
        options: vec![],
        selected: 0,
        responder: Some(tx),
    });
    let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert!(app.pending_question.is_none());
    assert_eq!(rx.blocking_recv().unwrap(), "[User declined to answer]");
}

#[test]
fn question_multichoice_number_selects() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_question = Some(PendingQuestion {
        question: "Color?".to_string(),
        tool_call_id: "c2".to_string(),
        options: vec!["red".to_string(), "green".to_string(), "blue".to_string()],
        selected: 0,
        responder: Some(tx),
    });
    // Press '2' for green
    let key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert!(app.pending_question.is_none());
    assert_eq!(rx.blocking_recv().unwrap(), "green");
}

#[test]
fn question_multichoice_arrows_navigate() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_question = Some(PendingQuestion {
        question: "Color?".to_string(),
        tool_call_id: "c3".to_string(),
        options: vec!["red".to_string(), "green".to_string()],
        selected: 0,
        responder: Some(tx),
    });
    let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
    app.update(Msg::Key(key));
    assert_eq!(app.pending_question.as_ref().unwrap().selected, 1);
}
```

**Step 2: Run tests — expect failures**

**Step 3: Implement handle_question_key()**

Port logic from current `src/tui/input.rs` `handle_question_key()` and `handle_multiple_choice_key()`:

```rust
impl ClawApp {
    fn handle_question_key(&mut self, key: KeyEvent) -> Command<Msg> {
        let has_options = self.pending_question.as_ref()
            .map_or(false, |q| !q.options.is_empty());

        if has_options {
            return self.handle_multichoice_key(key);
        }

        // Free-text mode: typing goes to TextArea, Enter sends, Esc dismisses
        match key.code {
            KeyCode::Enter => {
                let text = self.input.value().trim().to_string();
                if !text.is_empty() {
                    self.input.set_value("");
                    self.resolve_question(text);
                }
                Command::none()
            }
            KeyCode::Esc => {
                self.resolve_question("[User declined to answer]".to_string());
                Command::none()
            }
            _ => {
                // Forward to TextArea for normal editing
                self.input.update(text_area::Message::KeyPress(key)).map(Msg::Input)
            }
        }
    }

    fn handle_multichoice_key(&mut self, key: KeyEvent) -> Command<Msg> {
        let option_count = self.pending_question.as_ref()
            .map_or(0, |q| q.options.len());

        match key.code {
            KeyCode::Left => {
                if let Some(ref mut q) = self.pending_question {
                    q.selected = q.selected.saturating_sub(1);
                }
                Command::none()
            }
            KeyCode::Right => {
                if let Some(ref mut q) = self.pending_question {
                    q.selected = (q.selected + 1).min(option_count.saturating_sub(1));
                }
                Command::none()
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let idx = c.to_digit(10).unwrap_or(0) as usize;
                if idx >= 1 && idx <= option_count {
                    let answer = self.pending_question.as_ref()
                        .unwrap().options[idx - 1].clone();
                    self.resolve_question(answer);
                }
                Command::none()
            }
            KeyCode::Enter => {
                let answer = self.pending_question.as_ref()
                    .and_then(|q| q.options.get(q.selected).cloned());
                if let Some(answer) = answer {
                    self.resolve_question(answer);
                }
                Command::none()
            }
            KeyCode::Esc => {
                self.resolve_question("[User declined to answer]".to_string());
                Command::none()
            }
            _ => Command::none(),
        }
    }

    fn resolve_question(&mut self, answer: String) {
        if let Some(mut question) = self.pending_question.take() {
            if let Some(responder) = question.responder.take() {
                let _ = responder.send(answer);
            }
        }
    }
}
```

**Step 4: Run tests**

Run: `cargo test tui::model::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/tui/model.rs
git commit -m "feat: implement question mode handling in ClawApp"
```

---

### Task 8: Implement view()

**Files:**
- Modify: `src/tui/model.rs`

**Step 1: Write a basic render test**

```rust
#[test]
fn view_does_not_panic() {
    let (app, _) = ClawApp::init(test_flags());
    // Use ratatui's TestBackend to verify view doesn't panic
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.view(frame)).unwrap();
}

#[test]
fn view_with_messages_does_not_panic() {
    let (mut app, _) = ClawApp::init(test_flags());
    app.push_message(ChatMessageKind::User, "Hello".to_string());
    app.push_message(ChatMessageKind::Assistant, "World".to_string());
    app.streaming = true;
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.view(frame)).unwrap();
}

#[test]
fn view_with_approval_does_not_panic() {
    let (mut app, _) = ClawApp::init(test_flags());
    let (tx, _rx) = tokio::sync::oneshot::channel();
    app.pending_approval = Some(PendingApproval {
        description: "bash(ls)".to_string(),
        pattern: None,
        tool_name: "bash".to_string(),
        selected: 1,
        responder: Some(tx),
    });
    let backend = ratatui::backend::TestBackend::new(80, 24);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|frame| app.view(frame)).unwrap();
}
```

**Step 2: Run tests — expect failures**

**Step 3: Implement view()**

Port the rendering logic from current `src/tui/ui.rs` `render()`. The existing widget render functions (`render_chat_lines`, `approval_line`, `question_lines`, `multichoice_lines`, `status_line`) are called unchanged. Key differences:

1. **Input area**: Use `self.input.view(frame, input_chunk)` instead of manual Paragraph rendering
2. **Status bar**: Use boba's `StatusBar::new().left(...).center(...).right(...).render(frame, area)` OR keep using the existing `status_line()` function which returns a `Line` — render as `Paragraph::new(status_line(params))`
3. **Cursor**: TextArea manages its own cursor — remove manual `set_cursor_position` for normal input mode. Keep manual cursor positioning for question free-text mode if TextArea doesn't handle it.
4. **Input border**: TextArea renders its own block/border. If TextArea doesn't support custom border styling, render a border Block around the TextArea area and adjust.
5. **Scroll**: Same logic — `render_chat_lines` → `Paragraph` with `Wrap` → `line_count()` → scroll math

**Important**: The `view()` method takes `&self` (immutable). The current `render()` mutates `state.scroll_offset` to clamp it. In boba's model, we cannot mutate in view(). **Solution**: Clamp scroll_offset in update() (after any event that changes messages or scroll), not in view(). In view(), just use the pre-clamped value. Add a `clamp_scroll()` helper called at the end of relevant update arms.

For the initial implementation, keep the existing `status_line()` function instead of boba's StatusBar — it's simpler and already works.

**Note on TextArea rendering**: TextArea's `view(frame, area)` renders with its own borders and styling. Check if it matches our current look. If not, we may need to configure its style or render our own border and pass the inner area to TextArea.

**Step 4: Run tests**

Run: `cargo test tui::model::tests`
Expected: PASS

**Step 5: Commit**

```bash
git add src/tui/model.rs
git commit -m "feat: implement ClawApp view() with existing widgets"
```

---

### Task 9: Wire app.rs to Use boba::run_with

**Files:**
- Modify: `src/app.rs`

**Step 1: Rewrite App::run()**

Keep everything before the terminal setup (env loading, client creation, registry, MCP, approval engine, channels, system prompt, session logger, agent loop spawn). Remove:
- Manual terminal setup (`enable_raw_mode`, `EnterAlternateScreen`, etc.)
- Panic hook installation
- `TuiState::new()` creation
- The `event_loop()` call
- Manual terminal cleanup

Replace with:

```rust
use boba::{ProgramOptions, MouseMode};
use crate::tui::model::{ClawApp, Flags};

// ... (keep all setup code through agent loop spawn) ...

// Build session replay messages for the TUI.
let replay_messages = if let Some(ref session) = loaded_session {
    replay_session_messages(session)
} else {
    vec![]
};

// Build startup message.
let startup_message = build_startup_message(&context_file_names, &skill_file_names);

let flags = Flags {
    user_tx,
    agent_rx,
    model_name: model.clone(),
    tool_count,
    context_window: compaction::context_window_for_model(&model),
    workspace_dir: workspace_path.to_string_lossy().to_string(),
    replay_messages,
    startup_message,
};

let options = ProgramOptions {
    fps: 30,
    alt_screen: true,
    mouse_mode: Some(MouseMode::Normal),
    bracketed_paste: true,
    catch_panics: true,
    ..Default::default()
};

// Run the boba TUI — blocks until quit.
let result = boba::run_with::<ClawApp>(flags, options).await;

// Print farewell screen.
if let Ok(ref app) = result {
    print_exit_screen(app);
}

// Signal agent to quit and wait.
let _ = user_tx_for_quit.send(UserEvent::Quit).await;
drop(user_tx_for_quit);
let _ = agent_handle.await;

// Shutdown MCP clients.
for mcp_client in &mcp_clients {
    let _ = mcp_client.shutdown().await;
}

match result {
    Ok(_) => Ok(()),
    Err(e) => Err(anyhow::anyhow!("TUI error: {}", e)),
}
```

**Important detail**: `user_tx` is moved into Flags, but we also need it for the quit signal after boba exits. Solution: clone `user_tx` before moving into Flags, keep the clone for cleanup.

**Step 2: Extract helper functions**

- `replay_session_messages(session: &SessionState) -> Vec<ChatMessage>` — port the session replay loop from current app.rs:221-284
- `build_startup_message(context_files: &[String], skill_files: &[String]) -> String` — port from current app.rs:209-218
- `print_exit_screen(app: &ClawApp)` — port from current app.rs:386-403, using `app.session_start`, `app.messages.len()`

**Step 3: Remove old code from app.rs**

Delete:
- `event_loop()` method
- `handle_key_event()` function
- `handle_agent_event()` function
- `update_tool_status()` function
- `LoopAction` enum
- The `MOUSE_SCROLL_STEP` and `MAX_AGENT_EVENTS_PER_TICK` constants
- Old imports (`crossterm`, `ratatui::Terminal`, `ratatui::backend`, `TuiState`, `InputResult`, `handle_key`, `render`)

**Step 4: Verify build**

Run: `cargo build`
Expected: Compiles. Some warnings about unused old code are fine at this stage.

**Step 5: Run existing tests**

Run: `cargo test`
Expected: Old tests in app.rs that tested `handle_agent_event` and `update_tool_status` will fail — they reference deleted functions. These are now covered by the model.rs tests. Delete the old tests from app.rs.

**Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: wire app.rs to boba::run_with, remove manual event loop"
```

---

### Task 10: Clean Up Old Code

**Files:**
- Modify: `src/tui/state.rs` — remove TuiState struct and its impl, keep shared types
- Modify: `src/tui/mod.rs` — remove `pub mod input`, update re-exports
- Delete: `src/tui/input.rs` — logic now lives in model.rs update()
- Modify: `src/tui/ui.rs` — remove the `render()` function (now in model.rs view()), keep only if it has reusable helpers

**Step 1: Clean state.rs**

Remove from `src/tui/state.rs`:
- `TuiState` struct
- All `impl TuiState` methods
- `char_index_to_byte_index()` helper
- Tests for TuiState methods (cursor, input editing, etc.)

Keep:
- `ChatMessage`, `ChatMessageKind`, `ToolCallStatus`
- `AgentEvent`, `UserEvent`
- `PendingApproval`, `PendingQuestion`
- Tests for shared types (pending_question_lifecycle, pending_question_responder_sends)

**Step 2: Remove input.rs**

Delete `src/tui/input.rs` entirely. All key handling logic is now in `ClawApp::update()`.

Remove `pub mod input;` from `src/tui/mod.rs`.

**Step 3: Clean ui.rs**

If `render()` is the only function in ui.rs, the file can be deleted entirely. The widget render functions in `widgets/` are called directly from `ClawApp::view()`.

If ui.rs has reusable helpers beyond `render()`, keep those and delete only `render()`.

Remove `pub mod ui;` from `src/tui/mod.rs` if the file is deleted.

**Step 4: Update tui/mod.rs**

```rust
// ABOUTME: TUI module — boba-based Elm Architecture interface for soloclaw.
// ABOUTME: Chat display, input handling, status bar, and inline approval prompts.

pub mod model;
pub mod state;
pub mod subscriptions;
pub mod widgets;

pub use state::*;
```

**Step 5: Fix all compilation errors**

Run `cargo build` and fix any remaining references to deleted types or functions. Common fixes:
- Imports of `TuiState` → no longer needed (only `ClawApp` exists)
- Imports of `InputResult`, `handle_key` → deleted
- Imports of `render` from ui.rs → now in model.rs view()

**Step 6: Run full test suite**

Run: `cargo test`
Expected: All tests pass. Some test count will be lower (old TuiState tests removed, replaced by model.rs tests).

**Step 7: Commit**

```bash
git add -A
git commit -m "refactor: remove old TuiState, input.rs, and ui.rs render"
```

---

### Task 11: Final Verification and Polish

**Files:**
- Various small fixes

**Step 1: Full build and test**

Run: `cargo build && cargo test`
Expected: Clean build, all tests pass.

**Step 2: Check for warnings**

Run: `cargo build 2>&1 | grep warning`
Fix any unused import or dead code warnings.

**Step 3: Manual smoke test**

Run the application:
```bash
cargo run
```

Verify:
- TUI renders with header, chat area, input, status bar
- Typing works (characters appear in input)
- Enter sends messages
- Shift+Enter inserts newlines
- Esc quits
- Mouse scroll works
- If an LLM is configured: streaming text appears, tool calls show approval prompt

**Step 4: Verify TextArea behavior**

Test in the running app:
- Undo/redo (Ctrl+Z / Ctrl+Y)
- Word operations (Ctrl+W to delete word, Alt+D to delete word forward)
- Selection (Shift+arrows)
- Paste (Ctrl+V or bracket paste)

**Step 5: Verify session resume**

1. Send a message, quit
2. Relaunch — session should resume with history
3. `--fresh` should start clean

**Step 6: Final commit**

```bash
git add -A
git commit -m "chore: fix warnings and polish boba migration"
```

**Step 7: Run test suite one final time**

Run: `cargo test`
Expected: All tests pass with 0 failures.

---

## File Change Summary

| Action | File |
|---|---|
| Create | `src/tui/model.rs` |
| Create | `src/tui/subscriptions.rs` |
| Modify | `src/tui/mod.rs` |
| Modify | `src/tui/state.rs` (trim TuiState, keep shared types) |
| Modify | `src/app.rs` (rewrite to boba::run_with) |
| Modify | `Cargo.toml` (add boba, tokio-stream) |
| Delete | `src/tui/input.rs` |
| Delete or trim | `src/tui/ui.rs` |
| Unchanged | `src/agent/*`, `src/approval/*`, `src/session/*`, `src/tools/*`, `src/config.rs`, `src/prompt.rs` |
