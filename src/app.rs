// ABOUTME: App orchestrator — wires together LLM client, tools, approval, TUI, and agent loop.
// ABOUTME: Handles terminal setup/teardown, MCP connections, and the main event loop.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use mux::prelude::*;

use crate::agent;
use crate::approval::ApprovalEngine;
use crate::prompt::SystemPromptBuilder;
use crate::config::{load_mcp_configs, Config};
use crate::tui::input::{handle_key, InputResult};
use crate::tui::state::{
    AgentEvent, ChatMessageKind, PendingApproval, ToolCallStatus, TuiState, UserEvent,
};
use crate::tui::ui::render;

/// Top-level application that orchestrates all subsystems.
pub struct App {
    config: Config,
}

impl App {
    /// Create a new app with the given configuration.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Run the application: set up subsystems, launch the agent loop, and drive the TUI.
    pub async fn run(self) -> anyhow::Result<()> {
        // Load .env if present.
        let _ = dotenvy::dotenv();

        // Create LLM client.
        let client = agent::create_client(&self.config.llm)?;

        // Create tool registry and register built-in tools.
        let registry = Registry::new();
        registry.register(BashTool).await;
        registry.register(ReadFileTool).await;
        registry.register(WriteFileTool).await;
        registry.register(ListFilesTool).await;
        registry.register(SearchTool).await;

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
        let engine = Arc::new(ApprovalEngine::new(approvals_path)?);

        // Create channels for agent <-> TUI communication.
        let (user_tx, user_rx) = mpsc::channel::<UserEvent>(16);
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);

        let model = self.config.llm.model.clone();
        let max_tokens = self.config.llm.max_tokens;
        let tool_count = registry.count().await;

        // Build the system prompt from layered defaults + overrides.
        let system_prompt = {
            let mut builder = SystemPromptBuilder::new();
            builder.load_overrides().load_local();
            builder.build()
        };

        // Spawn the agent loop in a background task.
        let agent_handle = tokio::spawn(agent::run_agent_loop(
            client,
            registry,
            engine,
            model.clone(),
            max_tokens,
            system_prompt,
            user_rx,
            agent_tx,
        ));

        // Set up terminal.
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Set up panic hook to restore terminal on panic.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            original_hook(info);
        }));

        // Create TUI state.
        let mut state = TuiState::new(model, tool_count);

        // Run the event loop.
        let result = Self::event_loop(&mut terminal, &mut state, &user_tx, &mut agent_rx).await;

        // Cleanup terminal.
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

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

            // Poll for crossterm keyboard events (50ms timeout).
            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match handle_key_event(state, key, user_tx).await {
                        LoopAction::Continue => {}
                        LoopAction::Quit => break,
                    }
                }
            }

            // Drain all pending agent events.
            while let Ok(event) = agent_rx.try_recv() {
                match handle_agent_event(state, event) {
                    LoopAction::Continue => {}
                    LoopAction::Quit => break,
                }
            }
        }

        Ok(())
    }
}

/// Whether the event loop should continue or exit.
enum LoopAction {
    Continue,
    Quit,
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
        InputResult::Approval(_decision) => {
            // The approval resolution is handled inside handle_key via the oneshot channel.
            // We just need to clear the pending approval state (already done by handle_key).
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
        AgentEvent::Error(msg) => {
            state.push_message(ChatMessageKind::System, format!("Error: {}", msg));
            state.streaming = false;
        }
        AgentEvent::Done => {
            state.streaming = false;
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
}
