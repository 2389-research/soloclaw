// ABOUTME: Streaming agent loop — drives conversation between user, LLM, and tools.
// ABOUTME: Handles streaming responses, tool call approval/execution, and message history.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot, Mutex};

use mux::prelude::*;

use crate::agent::compaction;
use crate::approval::{ApprovalDecision, ApprovalEngine, EngineOutcome, ToolCallInfo};
use crate::config::CompactionConfig;
use crate::session::SessionLogger;
use crate::session::persistence::{SessionState, save_session};
use crate::tools::ask_user::ASK_USER_TOOL_NAME;
use crate::tui::state::{AgentEvent, UserEvent};

/// Metadata tracked for a tool call being assembled from streaming events.
struct PendingToolCall {
    id: String,
    name: String,
    json_buf: String,
}

/// Bundled parameters for the agent loop, replacing individual function arguments.
pub struct AgentLoopParams {
    pub client: Arc<dyn LlmClient>,
    pub registry: Registry,
    pub engine: Arc<ApprovalEngine>,
    pub model: String,
    pub max_tokens: u32,
    pub approval_timeout_seconds: u64,
    pub system_prompt: String,
    pub initial_messages: Vec<Message>,
    pub session_logger: Option<Arc<Mutex<SessionLogger>>>,
    pub workspace_dir: PathBuf,
    pub compaction_config: CompactionConfig,
    pub existing_created_at: Option<String>,
}

/// Log a message via the session logger, if one is configured.
async fn maybe_log_message(logger: &Option<Arc<Mutex<SessionLogger>>>, msg: &Message) {
    if let Some(logger) = logger {
        let mut guard = logger.lock().await;
        if let Err(e) = guard.log_message(msg) {
            eprintln!("Warning: failed to log session message: {}", e);
        }
    }
}

/// Run the agent loop, processing user messages and streaming LLM responses.
///
/// This function runs until the user sends a Quit event or the channel closes.
/// It manages conversation history, streams LLM responses to the TUI, handles
/// tool calls through the approval engine, and loops back to the LLM when
/// tool results are available.
pub async fn run_agent_loop(
    params: AgentLoopParams,
    mut user_rx: mpsc::Receiver<UserEvent>,
    agent_tx: mpsc::Sender<AgentEvent>,
) {
    let mut messages: Vec<Message> = params.initial_messages;
    let created_at = params
        .existing_created_at
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());

    loop {
        // Wait for a user event.
        let event = match user_rx.recv().await {
            Some(e) => e,
            None => break, // Channel closed.
        };

        match event {
            UserEvent::Quit => break,
            UserEvent::Message(text) => {
                let user_msg = Message::user(&text);
                maybe_log_message(&params.session_logger, &user_msg).await;
                messages.push(user_msg);

                // Enter the LLM conversation loop. After each round of tool calls,
                // we re-send the updated conversation to the LLM.
                if let Err(e) = conversation_turn(
                    &params.client,
                    &params.registry,
                    &params.engine,
                    &params.model,
                    params.max_tokens,
                    params.approval_timeout_seconds,
                    &params.system_prompt,
                    &mut messages,
                    &agent_tx,
                    &params.session_logger,
                )
                .await
                {
                    let _ = agent_tx.send(AgentEvent::Error(e.to_string())).await;
                }

                // Check if compaction is needed before signaling Done, so the
                // TUI keeps streaming=true and blocks user input during compaction.
                if compaction::needs_compaction(
                    &messages,
                    &params.model,
                    &params.compaction_config,
                ) {
                    let _ = agent_tx.send(AgentEvent::CompactionStarted).await;
                    let old_count = messages.len();

                    match compaction::run_compaction(
                        &params.client,
                        &params.model,
                        params.max_tokens,
                        &messages,
                    )
                    .await
                    {
                        Ok(summary_text) => {
                            let user_messages = compaction::collect_user_messages(&messages);
                            let compacted = compaction::build_compacted_history(
                                &user_messages,
                                &summary_text,
                                params.compaction_config.user_message_budget_tokens,
                            );
                            let new_count = compacted.len();
                            messages = compacted;
                            let _ = agent_tx
                                .send(AgentEvent::CompactionDone {
                                    old_count,
                                    new_count,
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = agent_tx
                                .send(AgentEvent::Error(format!("Compaction failed: {}", e)))
                                .await;
                        }
                    }
                }

                let _ = agent_tx.send(AgentEvent::Done).await;

                // Save session state after each complete turn.
                save_session(
                    &params.workspace_dir,
                    &SessionState {
                        workspace_dir: params.workspace_dir.to_string_lossy().to_string(),
                        model: params.model.clone(),
                        created_at: created_at.clone(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        messages: messages.clone(),
                        total_tokens: 0,
                    },
                )
                .ok();
            }
        }
    }
}

/// Execute one full conversation turn: stream LLM response, handle tool calls,
/// and loop back if the LLM stopped due to tool use.
async fn conversation_turn(
    client: &Arc<dyn LlmClient>,
    registry: &Registry,
    engine: &Arc<ApprovalEngine>,
    model: &str,
    max_tokens: u32,
    approval_timeout_seconds: u64,
    system_prompt: &str,
    messages: &mut Vec<Message>,
    agent_tx: &mpsc::Sender<AgentEvent>,
    session_logger: &Option<Arc<Mutex<SessionLogger>>>,
) -> anyhow::Result<()> {
    loop {
        let tool_defs = registry.to_definitions().await;

        let request = Request::new(model)
            .system(system_prompt)
            .max_tokens(max_tokens)
            .messages(messages.iter().cloned())
            .tools(tool_defs);

        let (assistant_blocks, stop_reason) = stream_response(client, &request, agent_tx).await?;

        // Record the assistant's response in conversation history.
        if !assistant_blocks.is_empty() {
            let assistant_msg = Message {
                role: Role::Assistant,
                content: assistant_blocks.clone(),
            };
            maybe_log_message(session_logger, &assistant_msg).await;
            messages.push(assistant_msg);
        }

        // If the LLM stopped because of tool use, execute tools and continue.
        if stop_reason == Some(StopReason::ToolUse) {
            let tool_results = execute_tool_calls(
                &assistant_blocks,
                registry,
                engine,
                approval_timeout_seconds,
                agent_tx,
            )
            .await;

            if !tool_results.is_empty() {
                let tool_msg = Message::tool_results(tool_results);
                maybe_log_message(session_logger, &tool_msg).await;
                messages.push(tool_msg);
            }

            // Loop back to send updated conversation to LLM.
            continue;
        }

        // End turn or max tokens — conversation turn is done.
        break;
    }

    Ok(())
}

/// Stream a single LLM response, forwarding text deltas and accumulating
/// content blocks (text + tool use). Returns the assembled content blocks
/// and the stop reason.
async fn stream_response(
    client: &Arc<dyn LlmClient>,
    request: &Request,
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> anyhow::Result<(Vec<ContentBlock>, Option<StopReason>)> {
    let mut stream = client.create_message_stream(request);

    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut pending_tools: HashMap<usize, PendingToolCall> = HashMap::new();
    let mut stop_reason: Option<StopReason> = None;
    let mut current_text = String::new();

    while let Some(event_result) = stream.next().await {
        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                let _ = agent_tx
                    .send(AgentEvent::Error(format!("Stream error: {}", e)))
                    .await;
                return Err(e.into());
            }
        };

        match event {
            StreamEvent::MessageStart { .. } => {}

            StreamEvent::ContentBlockStart { index, block } => {
                match &block {
                    ContentBlock::ToolUse { id, name, .. } => {
                        // Finalize any accumulated text before tool blocks.
                        if !current_text.is_empty() {
                            blocks.push(ContentBlock::text(&current_text));
                            let _ = agent_tx.send(AgentEvent::TextDone).await;
                            current_text.clear();
                        }
                        pending_tools.insert(
                            index,
                            PendingToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                json_buf: String::new(),
                            },
                        );
                    }
                    ContentBlock::Text { .. } => {
                        // Text block start — nothing special to do here.
                    }
                    _ => {}
                }
            }

            StreamEvent::ContentBlockDelta { index: _, text } => {
                current_text.push_str(&text);
                let _ = agent_tx.send(AgentEvent::TextDelta(text)).await;
            }

            StreamEvent::InputJsonDelta {
                index,
                partial_json,
            } => {
                if let Some(tool) = pending_tools.get_mut(&index) {
                    tool.json_buf.push_str(&partial_json);
                }
            }

            StreamEvent::ContentBlockStop { index } => {
                if let Some(tool) = pending_tools.remove(&index) {
                    let input: serde_json::Value = serde_json::from_str(&tool.json_buf)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                    blocks.push(ContentBlock::ToolUse {
                        id: tool.id,
                        name: tool.name,
                        input,
                    });
                }
                // If this was a text block, the text is already accumulated.
            }

            StreamEvent::MessageDelta {
                stop_reason: sr,
                usage,
            } => {
                if let Some(reason) = sr {
                    stop_reason = Some(reason);
                }
                let total = usage.input_tokens + usage.output_tokens;
                if total > 0 {
                    let _ = agent_tx
                        .send(AgentEvent::Usage {
                            input_tokens: usage.input_tokens,
                            output_tokens: usage.output_tokens,
                        })
                        .await;
                }
            }

            StreamEvent::MessageStop => {
                // Finalize any remaining text.
                if !current_text.is_empty() {
                    blocks.push(ContentBlock::text(&current_text));
                    let _ = agent_tx.send(AgentEvent::TextDone).await;
                    current_text.clear();
                }
            }
        }
    }

    // Handle case where stream ends without MessageStop.
    if !current_text.is_empty() {
        blocks.push(ContentBlock::text(&current_text));
        let _ = agent_tx.send(AgentEvent::TextDone).await;
    }

    Ok((blocks, stop_reason))
}

/// Execute all tool calls from the assistant's content blocks, routing through
/// the approval engine. Returns tool result content blocks to send back to the LLM.
async fn execute_tool_calls(
    assistant_blocks: &[ContentBlock],
    registry: &Registry,
    engine: &Arc<ApprovalEngine>,
    approval_timeout_seconds: u64,
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> Vec<ContentBlock> {
    let mut results = Vec::new();

    for block in assistant_blocks {
        let (id, name, input) = match block {
            ContentBlock::ToolUse { id, name, input } => (id, name, input),
            _ => continue,
        };

        // Intercept ask_user tool calls — bypass approval engine entirely.
        if name == ASK_USER_TOOL_NAME {
            let question = input
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("(no question provided)")
                .to_string();

            let options: Vec<String> = input
                .get("options")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            let (tx, rx) = oneshot::channel();
            let _ = agent_tx
                .send(AgentEvent::AskUser {
                    question,
                    tool_call_id: id.clone(),
                    options,
                    responder: tx,
                })
                .await;

            // Wait for user's answer (no timeout — user takes as long as they need).
            let answer = match rx.await {
                Ok(answer) => answer,
                Err(_) => "[No response received]".to_string(),
            };

            results.push(ContentBlock::tool_result(id, &answer));
            continue;
        }

        let params_summary = summarize_params(input);
        let _ = agent_tx
            .send(AgentEvent::ToolCallStarted {
                tool_name: name.clone(),
                params_summary,
            })
            .await;

        // Check approval.
        let info = ToolCallInfo {
            tool_name: name.clone(),
            params: input.clone(),
        };
        let outcome = engine.check(&info);

        match outcome {
            EngineOutcome::Allowed => {
                let _ = agent_tx
                    .send(AgentEvent::ToolCallApproved {
                        tool_name: name.clone(),
                    })
                    .await;

                let result = execute_single_tool(registry, name, input).await;
                send_tool_result(agent_tx, name, &result).await;
                results.push(tool_result_to_block(id, &result));
            }

            EngineOutcome::Denied { reason } => {
                let _ = agent_tx
                    .send(AgentEvent::ToolCallDenied {
                        tool_name: name.clone(),
                        reason: reason.clone(),
                    })
                    .await;
                results.push(ContentBlock::tool_error(id, format!("Denied: {}", reason)));
            }

            EngineOutcome::NeedsApproval {
                description,
                pattern,
            } => {
                let (tx, rx) = oneshot::channel();
                let _ = agent_tx
                    .send(AgentEvent::ToolCallNeedsApproval {
                        description,
                        pattern: pattern.clone(),
                        tool_name: name.clone(),
                        responder: tx,
                    })
                    .await;

                // Wait for user decision with timeout.
                let decision =
                    match tokio::time::timeout(Duration::from_secs(approval_timeout_seconds), rx)
                        .await
                    {
                        Ok(Ok(decision)) => decision,
                        Ok(Err(_)) => {
                            // Oneshot channel dropped — treat as deny.
                            ApprovalDecision::Deny
                        }
                        Err(_) => {
                            // Timeout — treat as deny.
                            ApprovalDecision::Deny
                        }
                    };

                // Record the decision in the engine for AllowAlways persistence.
                engine.resolve(name, pattern.as_deref(), decision);

                match decision {
                    ApprovalDecision::AllowOnce | ApprovalDecision::AllowAlways => {
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallApproved {
                                tool_name: name.clone(),
                            })
                            .await;

                        let result = execute_single_tool(registry, name, input).await;
                        send_tool_result(agent_tx, name, &result).await;
                        results.push(tool_result_to_block(id, &result));
                    }
                    ApprovalDecision::Deny => {
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallDenied {
                                tool_name: name.clone(),
                                reason: "denied by user".to_string(),
                            })
                            .await;
                        results.push(ContentBlock::tool_error(id, "Denied by user".to_string()));
                    }
                }
            }
        }
    }

    results
}

/// Execute a single tool by looking it up in the registry and calling its execute method.
async fn execute_single_tool(
    registry: &Registry,
    name: &str,
    input: &serde_json::Value,
) -> ToolResult {
    let tool = match registry.get(name).await {
        Some(t) => t,
        None => {
            return ToolResult::error(format!("Tool '{}' not found in registry", name));
        }
    };

    match tool.execute(input.clone()).await {
        Ok(result) => result,
        Err(e) => ToolResult::error(format!("Tool execution error: {}", e)),
    }
}

/// Send a tool result event to the TUI.
async fn send_tool_result(
    agent_tx: &mpsc::Sender<AgentEvent>,
    tool_name: &str,
    result: &ToolResult,
) {
    let _ = agent_tx
        .send(AgentEvent::ToolResult {
            tool_name: tool_name.to_string(),
            content: result.content.clone(),
            is_error: result.is_error,
        })
        .await;
}

/// Convert a ToolResult into a ContentBlock for the LLM conversation.
fn tool_result_to_block(tool_use_id: &str, result: &ToolResult) -> ContentBlock {
    if result.is_error {
        ContentBlock::tool_error(tool_use_id, &result.content)
    } else {
        ContentBlock::tool_result(tool_use_id, &result.content)
    }
}

/// Summarize tool parameters for display, truncating to 80 characters.
fn summarize_params(params: &serde_json::Value) -> String {
    let s = params.to_string();
    let truncated: String = s.chars().take(80).collect();
    if truncated.len() < s.len() {
        format!("{}...", truncated)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_short_params() {
        let params = serde_json::json!({"command": "ls"});
        let summary = summarize_params(&params);
        assert_eq!(summary, r#"{"command":"ls"}"#);
    }

    #[test]
    fn summarize_long_params_truncates() {
        let long = "x".repeat(200);
        let params = serde_json::json!({"command": long});
        let summary = summarize_params(&params);
        assert!(summary.len() <= 84); // 80 + "..."
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn tool_result_to_block_success() {
        let result = ToolResult::text("output");
        let block = tool_result_to_block("call-1", &result);
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "call-1");
                assert_eq!(content, "output");
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult block"),
        }
    }

    #[test]
    fn tool_result_to_block_error() {
        let result = ToolResult::error("something broke");
        let block = tool_result_to_block("call-2", &result);
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "call-2");
                assert_eq!(content, "something broke");
                assert!(is_error);
            }
            _ => panic!("expected ToolResult block"),
        }
    }

    #[test]
    fn agent_loop_params_is_constructible() {
        // Compile-time test: verify AgentLoopParams struct can be referenced
        // and its fields are accessible. We can't construct a full instance
        // without a real LlmClient, but we verify the type exists and field
        // names are correct.
        fn _check_fields(p: &AgentLoopParams) {
            let _: &Arc<dyn LlmClient> = &p.client;
            let _: &Registry = &p.registry;
            let _: &Arc<ApprovalEngine> = &p.engine;
            let _: &String = &p.model;
            let _: &u32 = &p.max_tokens;
            let _: &u64 = &p.approval_timeout_seconds;
            let _: &String = &p.system_prompt;
            let _: &Vec<Message> = &p.initial_messages;
            let _: &Option<Arc<Mutex<SessionLogger>>> = &p.session_logger;
            let _: &PathBuf = &p.workspace_dir;
            let _: &CompactionConfig = &p.compaction_config;
            let _: &Option<String> = &p.existing_created_at;
        }
    }
}
