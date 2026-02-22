// ABOUTME: Boba Model implementation — ClawApp is the Elm Architecture TUI.
// ABOUTME: All TUI state, message handling, and rendering lives here.

use std::sync::Arc;
use std::time::Instant;

use boba::widgets::text_area;
use boba::widgets::text_area::TextArea;
use boba::widgets::viewport::{self, Viewport};
use boba::{subscribe, terminal_events, Command, Component, Model, Subscription, TerminalEvent};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;
use tokio::sync::{mpsc, Mutex};

use crate::tui::widgets::approval::approval_line;
use crate::tui::widgets::chat::render_chat_lines;
use crate::tui::widgets::question::{multichoice_lines, question_lines};
use crate::tui::widgets::status::{StatusBarParams, status_line};

use crate::approval::ApprovalDecision;
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
    pub chat_viewport: Viewport,
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
    /// Timestamp of the last Ctrl+C press for double-tap quit detection.
    last_ctrl_c: Option<Instant>,
    user_tx: mpsc::Sender<UserEvent>,
    agent_rx: Arc<Mutex<Option<mpsc::Receiver<AgentEvent>>>>,
}

impl Model for ClawApp {
    type Message = Msg;
    type Flags = Flags;

    fn init(flags: Flags) -> (Self, Command<Msg>) {
        let mut input = TextArea::new().with_line_numbers(false).with_soft_wrap(true);
        input.focus();

        let mut app = ClawApp {
            input,
            messages: Vec::new(),
            chat_viewport: Viewport::new(""),
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
            last_ctrl_c: None,
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

        app.rebuild_chat_content();

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
                    self.chat_viewport.goto_bottom();
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
                    self.chat_viewport.goto_bottom();
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
                // Ctrl+Q always quits immediately.
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('q')
                {
                    return Command::quit();
                }

                // Double Ctrl+C within 500ms quits; single Ctrl+C just primes
                // the timer and clears the input as a "cancel" gesture.
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    let now = Instant::now();
                    if let Some(prev) = self.last_ctrl_c {
                        if now.duration_since(prev).as_millis() < 500 {
                            return Command::quit();
                        }
                    }
                    self.last_ctrl_c = Some(now);
                    // Single Ctrl+C cancels current input.
                    if !self.input.value().is_empty() {
                        self.input.set_value("");
                    }
                    return Command::none();
                }

                // Route to approval/question mode handlers when active
                if self.pending_approval.is_some() {
                    return self.handle_approval_key(key);
                }
                if self.pending_question.is_some() {
                    return self.handle_question_key(key);
                }

                match key.code {
                    KeyCode::PageUp => {
                        self.chat_viewport.update(viewport::Message::ScrollUp(10));
                        Command::none()
                    }
                    KeyCode::PageDown => {
                        self.chat_viewport.update(viewport::Message::ScrollDown(10));
                        Command::none()
                    }
                    KeyCode::Up if self.streaming => {
                        self.chat_viewport.update(viewport::Message::ScrollUp(1));
                        Command::none()
                    }
                    KeyCode::Down if self.streaming => {
                        self.chat_viewport.update(viewport::Message::ScrollDown(1));
                        Command::none()
                    }
                    KeyCode::Up => {
                        if self.input.cursor_row() == 0 {
                            self.chat_viewport.update(viewport::Message::ScrollUp(1));
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
                            self.chat_viewport.update(viewport::Message::ScrollDown(1));
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
                    self.chat_viewport.update(viewport::Message::ScrollUp(MOUSE_SCROLL_STEP));
                    Command::none()
                }
                MouseEventKind::ScrollDown => {
                    self.chat_viewport.update(viewport::Message::ScrollDown(MOUSE_SCROLL_STEP));
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

    fn view(&self, frame: &mut Frame) {
        let area = frame.area();
        let has_approval = self.pending_approval.is_some();
        let has_question = self.pending_question.is_some();

        // Maximum height the input area can grow to (in terminal rows).
        const MAX_INPUT_HEIGHT: u16 = 8;

        // Calculate input height based on visual line count (accounting for soft
        // wrap at terminal width). The inner width is the frame width minus 2 for
        // the left/right border cells.
        let input_height = if has_approval {
            3
        } else {
            let inner_width = area.width.saturating_sub(2).max(1) as usize;
            let visual_lines: usize = self
                .input
                .value()
                .split('\n')
                .map(|line| {
                    let w = unicode_width::UnicodeWidthStr::width(line);
                    if w == 0 { 1 } else { (w + inner_width - 1) / inner_width }
                })
                .sum();
            // +2 accounts for top and bottom borders
            (visual_lines as u16 + 2).clamp(3, MAX_INPUT_HEIGHT)
        };

        // Compute prompt area height dynamically so long questions/options wrap
        // instead of being truncated. Each logical Line is measured against the
        // terminal width to determine how many visual rows it occupies.
        let prompt_height = if has_approval {
            if let Some(ref approval) = self.pending_approval {
                let lines = approval_line(&approval.description, approval.selected);
                visual_line_height(&lines, area.width)
            } else {
                3
            }
        } else if has_question {
            if let Some(ref question) = self.pending_question {
                let lines = if question.options.is_empty() {
                    question_lines(&question.question)
                } else {
                    multichoice_lines(&question.question, &question.options, question.selected)
                };
                visual_line_height(&lines, area.width)
            } else {
                3
            }
        } else {
            0
        };

        // Dynamic layout: insert a dedicated prompt area when approval or question is pending.
        let constraints = if has_approval || has_question {
            vec![
                Constraint::Length(1),                   // Header
                Constraint::Min(3),                      // Chat area
                Constraint::Length(prompt_height as u16), // Approval/question prompt
                Constraint::Length(input_height),         // Input area
                Constraint::Length(1),                    // Status bar
            ]
        } else {
            vec![
                Constraint::Length(1),            // Header
                Constraint::Min(3),               // Chat area
                Constraint::Length(input_height),  // Input area
                Constraint::Length(1),             // Status bar
            ]
        };

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(area);

        // 1. Header (with debug key counter)
        let header = Line::from(vec![
            Span::styled(
                " \u{1f43e} claw",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(Paragraph::new(header), chunks[0]);

        // 2. Chat area — Viewport handles scrolling and rendering.
        self.chat_viewport.view(frame, chunks[1]);

        // 3. Approval or question prompt (only when pending)
        let (input_chunk, status_chunk) = if has_approval {
            if let Some(ref approval) = self.pending_approval {
                let approval_lines = approval_line(&approval.description, approval.selected);
                frame.render_widget(
                    Paragraph::new(approval_lines).wrap(Wrap { trim: false }),
                    chunks[2],
                );
            }
            (chunks[3], chunks[4])
        } else if has_question {
            if let Some(ref question) = self.pending_question {
                let q_lines = if question.options.is_empty() {
                    question_lines(&question.question)
                } else {
                    multichoice_lines(&question.question, &question.options, question.selected)
                };
                frame.render_widget(
                    Paragraph::new(q_lines).wrap(Wrap { trim: false }),
                    chunks[2],
                );
            }
            (chunks[3], chunks[4])
        } else {
            (chunks[2], chunks[3])
        };

        // 4. Input area
        if has_approval {
            // During approval: disabled input with yellow border.
            let input_block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(Color::Yellow));
            let inner = input_block.inner(input_chunk);
            frame.render_widget(input_block, input_chunk);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "(approve/deny the tool call above)",
                    Style::default().fg(Color::DarkGray),
                )),
                inner,
            );
        } else {
            // Render a block around the input area with streaming status in the title.
            let mut block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(Color::DarkGray));
            if self.streaming {
                let title = if self.queued_message.is_some() {
                    " \u{1f4e8} message queued "
                } else {
                    " \u{26a1} streaming... "
                };
                block = block.title(Span::styled(title, Style::default().fg(Color::DarkGray)));
            }
            let inner = block.inner(input_chunk);
            frame.render_widget(block, input_chunk);
            self.input.view(frame, inner);
        }

        // 5. Status bar
        let status = status_line(&StatusBarParams {
            workspace_dir: &self.workspace_dir,
            context_used: self.context_used,
            context_window: self.context_window,
            session_start: self.session_start,
            streaming: self.streaming,
        });
        frame.render_widget(Paragraph::new(status), status_chunk);
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
        self.rebuild_chat_content();
    }

    /// Append text to the last assistant message, or create a new one if needed.
    /// Keeps scroll pinned to the bottom so new content is always visible.
    pub fn append_to_last_assistant(&mut self, text: &str) {
        if let Some(msg) = self.messages.last_mut()
            && msg.kind == ChatMessageKind::Assistant
        {
            msg.content.push_str(text);
            self.rebuild_chat_content();
            return;
        }
        self.push_message(ChatMessageKind::Assistant, text.to_string());
    }

    /// Rebuild the viewport's styled content from current messages and scroll to bottom.
    fn rebuild_chat_content(&mut self) {
        self.chat_viewport.set_styled_content(render_chat_lines(&self.messages));
        self.chat_viewport.goto_bottom();
    }

    /// Update the status of the most recent tool call message matching the given tool name.
    fn update_tool_status(&mut self, tool_name: &str, new_status: ToolCallStatus) {
        for msg in self.messages.iter_mut().rev() {
            if let ChatMessageKind::ToolCall {
                tool_name: ref name,
                ref mut status,
            } = msg.kind
                && name == tool_name
            {
                *status = new_status;
                return;
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

    /// Handle key events while a tool approval prompt is active.
    fn handle_approval_key(&mut self, key: KeyEvent) -> Command<Msg> {
        match key.code {
            KeyCode::Left => {
                if let Some(ref mut approval) = self.pending_approval {
                    approval.selected = approval.selected.saturating_sub(1);
                }
                Command::none()
            }
            KeyCode::Right => {
                if let Some(ref mut approval) = self.pending_approval {
                    approval.selected = (approval.selected + 1).min(2);
                }
                Command::none()
            }
            KeyCode::Char('1') => self.resolve_approval(0),
            KeyCode::Char('2') => self.resolve_approval(1),
            KeyCode::Char('3') => self.resolve_approval(2),
            KeyCode::Enter => {
                let selected = self
                    .pending_approval
                    .as_ref()
                    .map_or(0, |a| a.selected);
                self.resolve_approval(selected)
            }
            _ => Command::none(),
        }
    }

    /// Resolve the pending approval by mapping the selected index to a decision
    /// and sending it via the oneshot channel.
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

    /// Handle key events while a question prompt is active.
    /// Dispatches to multichoice or free-text handling based on whether options exist.
    fn handle_question_key(&mut self, key: KeyEvent) -> Command<Msg> {
        let has_options = self
            .pending_question
            .as_ref()
            .is_some_and(|q| !q.options.is_empty());

        if has_options {
            return self.handle_multichoice_key(key);
        }

        // Free-text question mode
        match key.code {
            KeyCode::Enter => {
                let text = self.input.value();
                self.input.set_value("");
                self.resolve_question(text);
                Command::none()
            }
            KeyCode::Esc => {
                self.resolve_question("[User declined to answer]".to_string());
                Command::none()
            }
            _ => self
                .input
                .update(text_area::Message::KeyPress(key))
                .map(Msg::Input),
        }
    }

    /// Handle key events for multiple-choice question mode.
    fn handle_multichoice_key(&mut self, key: KeyEvent) -> Command<Msg> {
        match key.code {
            KeyCode::Left => {
                if let Some(ref mut q) = self.pending_question {
                    q.selected = q.selected.saturating_sub(1);
                }
                Command::none()
            }
            KeyCode::Right => {
                if let Some(ref mut q) = self.pending_question {
                    let max = q.options.len().saturating_sub(1);
                    q.selected = (q.selected + 1).min(max);
                }
                Command::none()
            }
            KeyCode::Enter => {
                let answer = self
                    .pending_question
                    .as_ref()
                    .and_then(|q| q.options.get(q.selected).cloned())
                    .unwrap_or_default();
                self.resolve_question(answer);
                Command::none()
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as usize) - ('1' as usize);
                let option_count = self
                    .pending_question
                    .as_ref()
                    .map_or(0, |q| q.options.len());
                if idx < option_count {
                    if let Some(ref mut q) = self.pending_question {
                        q.selected = idx;
                    }
                    let answer = self
                        .pending_question
                        .as_ref()
                        .and_then(|q| q.options.get(q.selected).cloned())
                        .unwrap_or_default();
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

    /// Resolve the pending question by sending the answer via the oneshot channel.
    fn resolve_question(&mut self, answer: String) {
        if let Some(mut question) = self.pending_question.take()
            && let Some(responder) = question.responder.take()
        {
            let _ = responder.send(answer);
        }
    }
}

/// Calculate how many terminal rows a set of styled Lines will occupy when
/// wrapped at the given width. Each Line's spans are measured by unicode
/// display width and ceiling-divided by the available width.
fn visual_line_height(lines: &[Line], width: u16) -> u16 {
    let w = width.max(1) as usize;
    lines
        .iter()
        .map(|line| {
            let line_width: usize = line
                .spans
                .iter()
                .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if line_width == 0 {
                1
            } else {
                ((line_width + w - 1) / w) as u16
            }
        })
        .sum()
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
        app.push_message(ChatMessageKind::User, "hello".to_string());
        // After push, viewport should be at bottom (auto-scroll)
        assert!(app.chat_viewport.at_bottom());
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
        assert!(app.chat_viewport.at_bottom());
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
    fn single_ctrl_c_clears_input_does_not_quit() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.input.set_value("some text");
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none(), "single Ctrl+C should not quit");
        assert_eq!(app.input.value(), "", "single Ctrl+C should clear input");
    }

    #[test]
    fn double_ctrl_c_quits() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        // First press primes the timer.
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
        // Second press within 500ms quits.
        let cmd = app.update(Msg::Key(key));
        assert!(!cmd.is_none(), "double Ctrl+C should quit");
    }

    #[test]
    fn ctrl_q_quits() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        let cmd = app.update(Msg::Key(key));
        assert!(!cmd.is_none(), "Ctrl+Q should quit immediately");
    }

    #[test]
    fn key_pageup_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
    }

    #[test]
    fn key_pagedown_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        let key = KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
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
        let cmd = app.update(Msg::Mouse(mouse));
        assert!(cmd.is_none());
    }

    #[test]
    fn mouse_scroll_down() {
        let (mut app, _) = ClawApp::init(test_flags());
        let mouse = crossterm::event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        let cmd = app.update(Msg::Mouse(mouse));
        assert!(cmd.is_none());
    }

    #[test]
    fn typing_character_appears_in_input() {
        let (mut app, _) = ClawApp::init(test_flags());
        // Verify focus is set
        assert!(app.input.focused(), "TextArea should be focused after init");
        // Type 'a'
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(
            app.input.value(),
            "a",
            "Typing 'a' should insert into TextArea"
        );
    }

    #[test]
    fn typing_multiple_characters() {
        let (mut app, _) = ClawApp::init(test_flags());
        for c in ['h', 'e', 'l', 'l', 'o'] {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            app.update(Msg::Key(key));
        }
        assert_eq!(app.input.value(), "hello");
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
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
        // Input cursor should still be at row 0 (scroll went to chat, not input)
    }

    #[test]
    fn key_up_during_streaming_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        let key = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
    }

    #[test]
    fn key_down_during_streaming_scrolls() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.streaming = true;
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
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
    fn non_actionable_key_during_pending_approval_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
        // Approval should still be pending
        assert!(app.pending_approval.is_some());
    }

    #[test]
    fn non_actionable_key_during_pending_question_returns_none() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "test?".to_string(),
            tool_call_id: "call-1".to_string(),
            options: vec!["a".to_string(), "b".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE);
        let cmd = app.update(Msg::Key(key));
        assert!(cmd.is_none());
        // Question should still be pending
        assert!(app.pending_question.is_some());
    }

    // --- Approval mode tests (Task 6) ---

    use crate::approval::ApprovalDecision;

    #[test]
    fn approval_enter_sends_allow_once() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "bash(ls)".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_approval.is_none());
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::AllowOnce);
    }

    #[test]
    fn approval_char_2_sends_allow_always() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_approval.is_none());
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::AllowAlways);
    }

    #[test]
    fn approval_char_3_sends_deny() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_approval.is_none());
        assert_eq!(rx.blocking_recv().unwrap(), ApprovalDecision::Deny);
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

    #[test]
    fn approval_left_arrow_clamps_at_zero() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Left, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.pending_approval.as_ref().unwrap().selected, 0);
    }

    #[test]
    fn approval_right_clamps_at_2() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_approval = Some(PendingApproval {
            description: "test".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 2,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Right, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.pending_approval.as_ref().unwrap().selected, 2);
    }

    // --- Question mode tests (Task 7) ---

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
        assert_eq!(app.input.value(), "");
    }

    #[test]
    fn question_freetext_esc_dismisses() {
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
    fn question_freetext_typing_goes_to_textarea() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Name?".to_string(),
            tool_call_id: "c1".to_string(),
            options: vec![],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Char('B'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.input.value().contains("B"));
        // Question should still be pending
        assert!(app.pending_question.is_some());
    }

    #[test]
    fn question_multichoice_enter_selects_first() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string(), "blue".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_question.is_none());
        assert_eq!(rx.blocking_recv().unwrap(), "red");
    }

    #[test]
    fn question_multichoice_number_key_selects() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string(), "blue".to_string()],
            selected: 0,
            responder: Some(tx),
        });
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

    #[test]
    fn question_multichoice_esc_dismisses() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_question.is_none());
        assert_eq!(rx.blocking_recv().unwrap(), "[User declined to answer]");
    }

    #[test]
    fn question_multichoice_typing_ignored() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert_eq!(app.input.value(), "");
        assert!(app.pending_question.is_some());
    }

    #[test]
    fn question_multichoice_number_out_of_range_ignored() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let key = KeyEvent::new(KeyCode::Char('5'), KeyModifiers::NONE);
        app.update(Msg::Key(key));
        assert!(app.pending_question.is_some());
    }

    // --- view() rendering tests (Task 8) ---

    #[test]
    fn view_does_not_panic() {
        let (app, _) = ClawApp::init(test_flags());
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

    #[test]
    fn view_with_question_does_not_panic() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Name?".to_string(),
            tool_call_id: "c1".to_string(),
            options: vec![],
            selected: 0,
            responder: Some(tx),
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.view(frame)).unwrap();
    }

    #[test]
    fn view_with_multichoice_does_not_panic() {
        let (mut app, _) = ClawApp::init(test_flags());
        let (tx, _rx) = tokio::sync::oneshot::channel();
        app.pending_question = Some(PendingQuestion {
            question: "Color?".to_string(),
            tool_call_id: "c2".to_string(),
            options: vec!["red".to_string(), "green".to_string()],
            selected: 0,
            responder: Some(tx),
        });
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.view(frame)).unwrap();
    }

    #[test]
    fn view_narrow_terminal_does_not_panic() {
        let (app, _) = ClawApp::init(test_flags());
        let backend = ratatui::backend::TestBackend::new(20, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.view(frame)).unwrap();
    }

    #[test]
    fn renders_user_message() {
        let (mut app, _) = ClawApp::init(test_flags());
        app.push_message(ChatMessageKind::User, "test input".to_string());
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.view(frame)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let content: String = buffer.content.iter().map(|c| c.symbol().to_string()).collect();
        assert!(content.contains("test input"), "Buffer should contain 'test input', got: {}", content);
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

    #[test]
    fn visual_line_height_short_line_is_one_row() {
        let lines = vec![Line::from("hello")];
        assert_eq!(visual_line_height(&lines, 80), 1);
    }

    #[test]
    fn visual_line_height_wraps_long_line() {
        // 20 chars in a 10-col terminal = 2 rows
        let lines = vec![Line::from("a]".repeat(10))];
        assert_eq!(visual_line_height(&lines, 10), 2);
    }

    #[test]
    fn visual_line_height_sums_multiple_lines() {
        let lines = vec![
            Line::from("short"),                     // 1 row at width 20
            Line::from("this is a longer string!!"), // 25 chars → 2 rows at width 20
        ];
        assert_eq!(visual_line_height(&lines, 20), 3);
    }

    #[test]
    fn visual_line_height_empty_line_counts_as_one() {
        let lines = vec![Line::from("")];
        assert_eq!(visual_line_height(&lines, 80), 1);
    }
}
