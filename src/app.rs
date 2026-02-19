// ABOUTME: App orchestrator — wires together LLM client, tools, approval, TUI, and agent loop.
// ABOUTME: Sets up subsystems then runs the boba TUI event loop.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};

use mux::prelude::*;

use boba::{MouseMode, ProgramOptions};

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
use crate::tui::model::{ClawApp, Flags};
use crate::tui::state::{ChatMessage, ChatMessageKind, ToolCallStatus, UserEvent};

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
        let (agent_tx, agent_rx) = mpsc::channel::<crate::tui::state::AgentEvent>(64);

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

        // Clone user_tx before moving it into Flags (need it for quit signal after boba exits).
        let user_tx_for_quit = user_tx.clone();

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
            mouse_mode: Some(MouseMode::CellMotion),
            catch_panics: true,
            ..Default::default()
        };

        // Run the boba TUI — blocks until quit.
        let result = boba::run_with::<ClawApp>(flags, options).await;

        // Print farewell screen.
        if let Ok(ref app) = result {
            print_exit_screen(app);
        }

        // Signal agent to quit and wait for it.
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
    }
}

/// Replay loaded session messages into ChatMessage format for the TUI.
fn replay_session_messages(session: &persistence::SessionState) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    for msg in &session.messages {
        match msg.role {
            Role::User => {
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            if !text.is_empty() {
                                messages.push(ChatMessage {
                                    kind: ChatMessageKind::User,
                                    content: text.clone(),
                                });
                            }
                        }
                        ContentBlock::ToolResult { content, is_error, .. } => {
                            messages.push(ChatMessage {
                                kind: ChatMessageKind::ToolResult { is_error: *is_error },
                                content: content.clone(),
                            });
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
                                messages.push(ChatMessage {
                                    kind: ChatMessageKind::Assistant,
                                    content: text.clone(),
                                });
                            }
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            let params_summary = input.to_string();
                            let truncated: String = params_summary.chars().take(80).collect();
                            let display = if truncated.len() < params_summary.len() {
                                format!("{}({}...)", name, truncated)
                            } else {
                                format!("{}({})", name, params_summary)
                            };
                            messages.push(ChatMessage {
                                kind: ChatMessageKind::ToolCall {
                                    tool_name: name.clone(),
                                    status: ToolCallStatus::Allowed,
                                },
                                content: display,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    messages
}

/// Build the startup system message showing loaded context and skill files.
fn build_startup_message(context_file_names: &[String], skill_file_names: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if context_file_names.is_empty() {
        parts.push("No context files found".to_string());
    } else {
        parts.push(format!("Context: {}", context_file_names.join(", ")));
    }
    if !skill_file_names.is_empty() {
        parts.push(format!("Skills: {}", skill_file_names.join(", ")));
    }
    parts.join(" | ")
}

/// Print a farewell screen after the TUI exits.
fn print_exit_screen(app: &ClawApp) {
    let elapsed_secs = app.session_start.elapsed().as_secs();
    let elapsed = if elapsed_secs >= 3600 {
        format!("{}h {:02}m", elapsed_secs / 3600, (elapsed_secs % 3600) / 60)
    } else {
        format!("{}m {:02}s", elapsed_secs / 60, elapsed_secs % 60)
    };
    let msg_count = app.messages.len();

    println!();
    println!("  \u{1f43e} \x1b[1mThanks for using claw!\x1b[0m");
    println!();
    println!("  \u{2728} You showed up for AI today, and that's pretty cool.");
    println!("  \u{1f550} Session lasted {elapsed} with {msg_count} messages exchanged.");
    println!();
    println!("  \u{1f49c} Until next time \u{2014} keep building awesome things!");
    println!();
}
