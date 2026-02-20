// ABOUTME: App orchestrator — wires together LLM client, tools, approval, TUI, and agent loop.
// ABOUTME: Handles terminal setup/teardown, MCP connections, and the main event loop.

use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::{mpsc, Mutex};

use mux::prelude::*;

use crate::agent;
use crate::agent::AgentLoopParams;
use crate::agent::compaction;
use crate::approval::ApprovalEngine;
use crate::tools::ask_user::AskUserTool;
use crate::config::{Config, load_mcp_configs};
use crate::prompt::{
    SystemPromptParams, build_system_prompt, load_context_files, load_skill_files,
};
use crate::session::SessionLogger;
use crate::session::persistence;
use crate::tui::input::{InputResult, handle_key};
use crate::tui::state::{
    AgentEvent, ChatMessageKind, PendingApproval, PendingQuestion, ToolCallStatus, TuiState,
    UserEvent,
};

use crate::tui::ui::render;

const MOUSE_SCROLL_STEP: u16 = 3;
const MAX_AGENT_EVENTS_PER_TICK: usize = 128;

/// Top-level application that orchestrates all subsystems.
pub struct App {
    config: Config,
    fresh: bool,
}

impl App {
    /// Create a new app with the given configuration.
    pub fn new(config: Config, fresh: bool) -> Self {
        Self { config, fresh }
    }

    /// Run the application: set up subsystems, launch the agent loop, and drive the TUI.
    pub async fn run(self) -> anyhow::Result<()> {
        // Load local .env if present, then XDG secrets.
        let _ = dotenvy::dotenv();
        let _ = dotenvy::from_path(Config::secrets_env_path());

        // Create LLM client.
        let client = agent::create_client(&self.config.llm)?;

        // Create tool registry and register built-in tools.
        let registry = Registry::new();
        registry.register(BashTool).await;
        registry.register(ReadFileTool).await;
        registry.register(WriteFileTool).await;
        registry.register(ListFilesTool).await;
        registry.register(SearchTool).await;
        registry.register(AskUserTool).await;

        // Connect MCP servers.
        let mcp_configs = load_mcp_configs()?;
        let mut mcp_clients: Vec<Arc<McpClient>> = Vec::new();
        for mcp_config in mcp_configs {
            let name = mcp_config.name.clone();
            match McpClient::connect(mcp_config).await {
                Ok(mut mcp_client) => {
                    if let Err(e) = mcp_client.initialize().await {
                        eprintln!("Warning: failed to initialize MCP server '{}': {}", name, e);
                        continue;
                    }
                    let mcp_client = Arc::new(mcp_client);
                    if let Err(e) = registry.merge_mcp(mcp_client.clone(), Some(&name)).await {
                        eprintln!("Warning: failed to merge MCP tools from '{}': {}", name, e);
                    }
                    mcp_clients.push(mcp_client);
                }
                Err(e) => {
                    eprintln!("Warning: failed to connect MCP server '{}': {}", name, e);
                }
            }
        }

        // Create approval engine.
        let approvals_path = Config::approvals_path();
        let engine = Arc::new(ApprovalEngine::new_with_bypass(
            approvals_path,
            self.config.permissions.bypass_approvals,
        )?);

        // Create channels for agent <-> TUI communication.
        let (user_tx, user_rx) = mpsc::channel::<UserEvent>(16);
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);

        let model = self.config.llm.model.clone();
        let max_tokens = self.config.llm.max_tokens;
        let approval_timeout_seconds = self.config.approval.timeout_seconds;
        let tool_count = registry.count().await;

        // Gather runtime info and build the system prompt.
        let workspace_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let workspace_dir = workspace_path.to_string_lossy().to_string();

        let context_files = load_context_files(&workspace_dir);
        let skill_files = load_skill_files(&workspace_dir, &self.config.skills);

        // Collect context file names for the startup message shown in the TUI.
        let context_file_names: Vec<String> =
            context_files.iter().map(|f| f.path.clone()).collect();
        let skill_file_names: Vec<String> =
            skill_files.iter().map(|f| f.name.clone()).collect();

        // Collect tool names and summaries from the registry.
        let tool_defs = registry.to_definitions().await;
        let tool_names: Vec<String> = tool_defs.iter().map(|d| d.name.clone()).collect();
        let tool_summaries: std::collections::HashMap<String, String> = tool_defs
            .iter()
            .map(|d| (d.name.clone(), d.description.clone()))
            .collect();

        let system_prompt = build_system_prompt(&SystemPromptParams {
            tool_names,
            tool_summaries,
            workspace_dir,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            shell: std::env::var("SHELL").unwrap_or_default(),
            model: model.clone(),
            context_files,
            skill_files,
        });

        // Create session logger for conversation persistence.
        let session_logger = match SessionLogger::new(&workspace_path) {
            Ok(logger) => Some(Arc::new(Mutex::new(logger))),
            Err(e) => {
                eprintln!("Warning: failed to create session logger: {}", e);
                None
            }
        };

        // Try to load an existing session for this workspace (unless --fresh).
        let loaded_session = if !self.fresh {
            persistence::load_session(&workspace_path).ok().flatten()
        } else {
            None
        };

        let initial_messages = loaded_session
            .as_ref()
            .map(|s| s.messages.clone())
            .unwrap_or_default();

        // Spawn the agent loop in a background task.
        let agent_handle = tokio::spawn(agent::run_agent_loop(
            AgentLoopParams {
                client,
                registry,
                engine,
                model: model.clone(),
                max_tokens,
                approval_timeout_seconds,
                system_prompt,
                initial_messages,
                session_logger,
                workspace_dir: workspace_path.clone(),
                compaction_config: self.config.compaction.clone(),
                existing_created_at: loaded_session.as_ref().map(|s| s.created_at.clone()),
            },
            user_rx,
            agent_tx,
        ));

        // Set up terminal.
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Set up panic hook to restore terminal on panic.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste);
            original_hook(info);
        }));

        // Create TUI state.
        let mut state = TuiState::new(model.clone(), tool_count);
        state.context_window = compaction::context_window_for_model(&model);
        state.workspace_dir = workspace_path.to_string_lossy().to_string();

        // Show a startup message listing loaded context and skill files.
        let mut startup_parts: Vec<String> = Vec::new();
        if context_file_names.is_empty() {
            startup_parts.push("No context files found".to_string());
        } else {
            startup_parts.push(format!("Context: {}", context_file_names.join(", ")));
        }
        if !skill_file_names.is_empty() {
            startup_parts.push(format!("Skills: {}", skill_file_names.join(", ")));
        }
        state.push_message(ChatMessageKind::System, startup_parts.join(" | "));

        // Replay loaded session messages into the TUI for display.
        if let Some(ref session) = loaded_session {
            for msg in &session.messages {
                match msg.role {
                    Role::User => {
                        for block in &msg.content {
                            match block {
                                ContentBlock::Text { text } => {
                                    if !text.is_empty() {
                                        state.push_message(ChatMessageKind::User, text.clone());
                                    }
                                }
                                ContentBlock::ToolResult {
                                    content, is_error, ..
                                } => {
                                    state.push_message(
                                        ChatMessageKind::ToolResult {
                                            is_error: *is_error,
                                        },
                                        content.clone(),
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    Role::Assistant => {
                        for block in &msg.content {
                            match block {
                                ContentBlock::Text { text } => {
                                    if !text.is_empty() {
                                        state.push_message(
                                            ChatMessageKind::Assistant,
                                            text.clone(),
                                        );
                                    }
                                }
                                ContentBlock::ToolUse { name, input, .. } => {
                                    let params_summary = input.to_string();
                                    let truncated: String =
                                        params_summary.chars().take(80).collect();
                                    let display = if truncated.len() < params_summary.len() {
                                        format!("{}({}...)", name, truncated)
                                    } else {
                                        format!("{}({})", name, params_summary)
                                    };
                                    state.push_message(
                                        ChatMessageKind::ToolCall {
                                            tool_name: name.clone(),
                                            status: ToolCallStatus::Allowed,
                                        },
                                        display,
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            state.push_message(
                ChatMessageKind::System,
                "🔄 Session resumed".to_string(),
            );
        }

        // Run the event loop.
        let result = Self::event_loop(&mut terminal, &mut state, &user_tx, &mut agent_rx).await;

        // Cleanup terminal.
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;

        // Print farewell screen.
        Self::print_exit_screen(&state);

        // Signal agent to quit and wait for it.
        let _ = user_tx.send(UserEvent::Quit).await;
        drop(user_tx);
        let _ = agent_handle.await;

        // Shutdown MCP clients.
        for mcp_client in &mcp_clients {
            let _ = mcp_client.shutdown().await;
        }

        result
    }

    /// Main event loop: draw TUI, poll for keyboard input, drain agent events.
    async fn event_loop(
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        state: &mut TuiState,
        user_tx: &mpsc::Sender<UserEvent>,
        agent_rx: &mut mpsc::Receiver<AgentEvent>,
    ) -> anyhow::Result<()> {
        loop {
            // Draw the current state.
            terminal.draw(|frame| render(frame, state))?;

            // Wait for at least one terminal event (50ms timeout).
            if event::poll(Duration::from_millis(50))? {
                // Drain ALL pending terminal events before redrawing.
                // Without this, mouse motion events from EnableMouseCapture
                // flood the queue and starve keyboard input.
                loop {
                    let quit = Self::process_terminal_event(
                        event::read()?,
                        state,
                        user_tx,
                    )
                    .await;
                    if quit {
                        return Ok(());
                    }
                    // Keep draining while more events are immediately available.
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }

            // Drain a bounded number of pending agent events so user input stays responsive.
            let mut queued_send: Option<String> = None;
            for _ in 0..MAX_AGENT_EVENTS_PER_TICK {
                let Ok(event) = agent_rx.try_recv() else {
                    break;
                };
                match handle_agent_event(state, event) {
                    LoopAction::Continue => {}
                    LoopAction::Quit => break,
                    LoopAction::SendQueued(text) => {
                        queued_send = Some(text);
                        break;
                    }
                }
            }
            // Auto-send any queued message after the drain loop completes.
            if let Some(text) = queued_send {
                state.push_message(ChatMessageKind::User, text.clone());
                state.streaming = true;
                let _ = user_tx.send(UserEvent::Message(text)).await;
            }
        }
    }

    /// Handle a single terminal event. Returns true if the loop should quit.
    async fn process_terminal_event(
        event: Event,
        state: &mut TuiState,
        user_tx: &mpsc::Sender<UserEvent>,
    ) -> bool {
        match event {
            Event::Key(key) => match handle_key_event(state, key, user_tx).await {
                LoopAction::Continue => {}
                LoopAction::Quit => return true,
                LoopAction::SendQueued(text) => {
                    state.push_message(ChatMessageKind::User, text.clone());
                    state.streaming = true;
                    let _ = user_tx.send(UserEvent::Message(text)).await;
                }
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => {
                    state.scroll_offset =
                        state.scroll_offset.saturating_add(MOUSE_SCROLL_STEP);
                }
                MouseEventKind::ScrollDown => {
                    state.scroll_offset =
                        state.scroll_offset.saturating_sub(MOUSE_SCROLL_STEP);
                }
                _ => {}
            },
            Event::Paste(text) => {
                if !state.has_pending_approval() {
                    // Allow pasting in normal input, question mode, and streaming.
                    state.insert_str_at_cursor(&text);
                }
            }
            _ => {}
        }
        false
    }

    /// Print a farewell screen after the TUI exits.
    fn print_exit_screen(state: &TuiState) {
        let elapsed_secs = state.session_start.elapsed().as_secs();
        let elapsed = if elapsed_secs >= 3600 {
            format!("{}h {:02}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
        } else {
            format!("{}m {:02}s", elapsed_secs / 60, elapsed_secs % 60)
        };
        let msg_count = state.messages.len();

        println!();
        println!("  🐾 \x1b[1mThanks for hanging out with soloclaw!\x1b[0m");
        println!();
        println!("  ✨ You showed up for AI today, and that's pretty cool.");
        println!("  🕐 Session lasted {elapsed} with {msg_count} messages exchanged.");
        println!();
        println!("  💜 Until next time — keep building awesome things!");
        println!();
    }
}

/// Whether the event loop should continue or exit.
enum LoopAction {
    Continue,
    Quit,
    /// Auto-send a queued message that was typed during streaming.
    SendQueued(String),
}

/// Process a keyboard event and potentially send a message to the agent.
async fn handle_key_event(
    state: &mut TuiState,
    key: KeyEvent,
    user_tx: &mpsc::Sender<UserEvent>,
) -> LoopAction {
    match handle_key(state, key) {
        InputResult::None => LoopAction::Continue,
        InputResult::Send(text) => {
            state.push_message(ChatMessageKind::User, text.clone());
            state.streaming = true;
            let _ = user_tx.send(UserEvent::Message(text)).await;
            LoopAction::Continue
        }
        InputResult::Queue(text) => {
            state.queued_message = Some(text);
            LoopAction::Continue
        }
        InputResult::Approval(_decision) => {
            // The approval resolution is handled inside handle_key via the oneshot channel.
            // We just need to clear the pending approval state (already done by handle_key).
            LoopAction::Continue
        }
        InputResult::QuestionAnswered(_answer) => {
            // The question resolution is handled inside handle_key via the oneshot channel.
            // We just need to clear the pending question state (already done by handle_key).
            LoopAction::Continue
        }
        InputResult::Quit => LoopAction::Quit,
    }
}

/// Process an agent event and update the TUI state accordingly.
fn handle_agent_event(state: &mut TuiState, event: AgentEvent) -> LoopAction {
    match event {
        AgentEvent::TextDelta(text) => {
            state.append_to_last_assistant(&text);
        }
        AgentEvent::TextDone => {
            // Text streaming for this block is done; nothing special needed.
        }
        AgentEvent::ToolCallStarted {
            tool_name,
            params_summary,
        } => {
            let content = format!("{}({})", tool_name, params_summary);
            state.push_message(
                ChatMessageKind::ToolCall {
                    tool_name,
                    status: ToolCallStatus::Pending,
                },
                content,
            );
        }
        AgentEvent::ToolCallApproved { tool_name } => {
            // Update the last tool call message for this tool to show Allowed status.
            update_tool_status(state, &tool_name, ToolCallStatus::Allowed);
        }
        AgentEvent::ToolCallNeedsApproval {
            description,
            pattern,
            tool_name,
            responder,
        } => {
            state.pending_approval = Some(PendingApproval {
                description,
                pattern,
                tool_name,
                selected: 0,
                responder: Some(responder),
            });
            state.scroll_offset = 0;
        }
        AgentEvent::AskUser {
            question,
            tool_call_id,
            options,
            responder,
        } => {
            state.pending_question = Some(PendingQuestion {
                question,
                tool_call_id,
                options,
                selected: 0,
                responder: Some(responder),
            });
            state.scroll_offset = 0;
        }
        AgentEvent::ToolCallDenied { tool_name, reason } => {
            update_tool_status(state, &tool_name, ToolCallStatus::Denied);
            state.push_message(
                ChatMessageKind::System,
                format!("Tool '{}' denied: {}", tool_name, reason),
            );
        }
        AgentEvent::ToolResult {
            tool_name: _,
            content,
            is_error,
        } => {
            state.push_message(ChatMessageKind::ToolResult { is_error }, content);
        }
        AgentEvent::Usage {
            input_tokens,
            output_tokens,
        } => {
            state.total_tokens += (input_tokens + output_tokens) as u64;
            state.context_used = input_tokens as u64;
        }
        AgentEvent::Error(msg) => {
            state.push_message(ChatMessageKind::System, format!("⚠️ Error: {}", msg));
            state.streaming = false;
        }
        AgentEvent::Done => {
            state.streaming = false;
            // Auto-send any queued message from the user.
            if let Some(queued) = state.queued_message.take() {
                return LoopAction::SendQueued(queued);
            }
        }
        AgentEvent::CompactionStarted => {
            state.push_message(
                ChatMessageKind::System,
                "🗜️ Compacting conversation...".to_string(),
            );
        }
        AgentEvent::CompactionDone {
            old_count,
            new_count,
        } => {
            state.push_message(
                ChatMessageKind::System,
                format!(
                    "✅ Compacted: {} messages \u{2192} {} messages",
                    old_count, new_count
                ),
            );
        }
    }

    LoopAction::Continue
}

/// Update the status of the most recent tool call message matching the given tool name.
fn update_tool_status(state: &mut TuiState, tool_name: &str, new_status: ToolCallStatus) {
    for msg in state.messages.iter_mut().rev() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_agent_text_delta_appends() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(&mut state, AgentEvent::TextDelta("Hello".to_string()));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "Hello");
        assert_eq!(state.messages[0].kind, ChatMessageKind::Assistant);

        handle_agent_event(&mut state, AgentEvent::TextDelta(" world".to_string()));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].content, "Hello world");
    }

    #[test]
    fn handle_agent_done_stops_streaming() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.streaming = true;
        handle_agent_event(&mut state, AgentEvent::Done);
        assert!(!state.streaming);
    }

    #[test]
    fn handle_agent_error_stops_streaming() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.streaming = true;
        handle_agent_event(
            &mut state,
            AgentEvent::Error("something went wrong".to_string()),
        );
        assert!(!state.streaming);
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].content.contains("something went wrong"));
    }

    #[test]
    fn handle_agent_tool_call_started() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallStarted {
                tool_name: "bash".to_string(),
                params_summary: r#"{"command":"ls"}"#.to_string(),
            },
        );
        assert_eq!(state.messages.len(), 1);
        match &state.messages[0].kind {
            ChatMessageKind::ToolCall { tool_name, status } => {
                assert_eq!(tool_name, "bash");
                assert_eq!(*status, ToolCallStatus::Pending);
            }
            _ => panic!("expected ToolCall message"),
        }
    }

    #[test]
    fn handle_agent_tool_approved_updates_status() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallStarted {
                tool_name: "bash".to_string(),
                params_summary: "{}".to_string(),
            },
        );
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallApproved {
                tool_name: "bash".to_string(),
            },
        );
        match &state.messages[0].kind {
            ChatMessageKind::ToolCall { status, .. } => {
                assert_eq!(*status, ToolCallStatus::Allowed);
            }
            _ => panic!("expected ToolCall message"),
        }
    }

    #[test]
    fn handle_agent_tool_denied() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallStarted {
                tool_name: "bash".to_string(),
                params_summary: "{}".to_string(),
            },
        );
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallDenied {
                tool_name: "bash".to_string(),
                reason: "not allowed".to_string(),
            },
        );
        // Tool call status should be Denied.
        match &state.messages[0].kind {
            ChatMessageKind::ToolCall { status, .. } => {
                assert_eq!(*status, ToolCallStatus::Denied);
            }
            _ => panic!("expected ToolCall message"),
        }
        // System message about denial.
        assert_eq!(state.messages.len(), 2);
        assert!(state.messages[1].content.contains("not allowed"));
    }

    #[test]
    fn handle_agent_tool_result() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(
            &mut state,
            AgentEvent::ToolResult {
                tool_name: "bash".to_string(),
                content: "file1.txt\nfile2.txt".to_string(),
                is_error: false,
            },
        );
        assert_eq!(state.messages.len(), 1);
        match &state.messages[0].kind {
            ChatMessageKind::ToolResult { is_error } => {
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult message"),
        }
    }

    #[test]
    fn handle_agent_needs_approval_sets_pending() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        let (tx, _rx) = tokio::sync::oneshot::channel();
        handle_agent_event(
            &mut state,
            AgentEvent::ToolCallNeedsApproval {
                description: "bash(rm -rf /)".to_string(),
                pattern: Some("/usr/bin/rm".to_string()),
                tool_name: "bash".to_string(),
                responder: tx,
            },
        );
        assert!(state.has_pending_approval());
        let approval = state.pending_approval.as_ref().unwrap();
        assert_eq!(approval.tool_name, "bash");
        assert_eq!(approval.description, "bash(rm -rf /)");
        assert_eq!(approval.selected, 0);
    }

    #[test]
    fn update_tool_status_finds_last_matching() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.push_message(
            ChatMessageKind::ToolCall {
                tool_name: "bash".to_string(),
                status: ToolCallStatus::Pending,
            },
            "first".to_string(),
        );
        state.push_message(ChatMessageKind::Assistant, "some text".to_string());
        state.push_message(
            ChatMessageKind::ToolCall {
                tool_name: "bash".to_string(),
                status: ToolCallStatus::Pending,
            },
            "second".to_string(),
        );

        update_tool_status(&mut state, "bash", ToolCallStatus::Allowed);

        // The second (last) tool call should be updated.
        match &state.messages[2].kind {
            ChatMessageKind::ToolCall { status, .. } => {
                assert_eq!(*status, ToolCallStatus::Allowed);
            }
            _ => panic!("expected ToolCall"),
        }
        // The first should remain Pending.
        match &state.messages[0].kind {
            ChatMessageKind::ToolCall { status, .. } => {
                assert_eq!(*status, ToolCallStatus::Pending);
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn handle_agent_ask_user_sets_pending_question() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.scroll_offset = 5;
        let (tx, _rx) = tokio::sync::oneshot::channel();
        handle_agent_event(
            &mut state,
            AgentEvent::AskUser {
                question: "What is your name?".to_string(),
                tool_call_id: "call-42".to_string(),
                options: vec![],
                responder: tx,
            },
        );
        assert!(state.has_pending_question());
        let q = state.pending_question.as_ref().unwrap();
        assert_eq!(q.question, "What is your name?");
        assert_eq!(q.tool_call_id, "call-42");
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn handle_agent_ask_user_with_options() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        let (tx, _rx) = tokio::sync::oneshot::channel();
        handle_agent_event(
            &mut state,
            AgentEvent::AskUser {
                question: "Pick a color".to_string(),
                tool_call_id: "call-mc".to_string(),
                options: vec!["red".to_string(), "green".to_string(), "blue".to_string()],
                responder: tx,
            },
        );
        assert!(state.has_pending_question());
        let q = state.pending_question.as_ref().unwrap();
        assert_eq!(q.options.len(), 3);
        assert_eq!(q.options[0], "red");
        assert_eq!(q.selected, 0);
    }

    #[test]
    fn handle_agent_ask_user_responder_is_set() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle_agent_event(
            &mut state,
            AgentEvent::AskUser {
                question: "pick a color".to_string(),
                tool_call_id: "call-99".to_string(),
                options: vec![],
                responder: tx,
            },
        );
        // Verify the responder is present and can send
        let q = state.pending_question.take().unwrap();
        q.responder.unwrap().send("blue".to_string()).unwrap();
        assert_eq!(rx.blocking_recv().unwrap(), "blue");
    }

    #[test]
    fn handle_agent_done_sends_queued_message() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.streaming = true;
        state.queued_message = Some("follow up".to_string());
        let action = handle_agent_event(&mut state, AgentEvent::Done);
        assert!(!state.streaming);
        assert!(state.queued_message.is_none());
        match action {
            LoopAction::SendQueued(text) => assert_eq!(text, "follow up"),
            _ => panic!("expected SendQueued"),
        }
    }

    #[test]
    fn handle_agent_done_no_queue_continues() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        state.streaming = true;
        let action = handle_agent_event(&mut state, AgentEvent::Done);
        assert!(!state.streaming);
        assert!(matches!(action, LoopAction::Continue));
    }

    #[test]
    fn handle_agent_compaction_started() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(&mut state, AgentEvent::CompactionStarted);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].kind, ChatMessageKind::System);
        assert_eq!(state.messages[0].content, "🗜️ Compacting conversation...");
    }

    #[test]
    fn handle_agent_compaction_done() {
        let mut state = TuiState::new("test-model".to_string(), 3);
        handle_agent_event(
            &mut state,
            AgentEvent::CompactionDone {
                old_count: 50,
                new_count: 5,
            },
        );
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].kind, ChatMessageKind::System);
        assert!(state.messages[0].content.contains("50"));
        assert!(state.messages[0].content.contains("5"));
    }
}
