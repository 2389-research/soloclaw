// ABOUTME: Streaming agent loop — drives conversation between user, LLM, and tools.
// ABOUTME: Handles streaming responses, tool call approval/execution, and message history.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};

use mux::prelude::*;

use crate::approval::{ApprovalDecision, ApprovalEngine, EngineOutcome, ToolCallInfo};
use crate::tui::state::{AgentEvent, UserEvent};

/// Metadata tracked for a tool call being assembled from streaming events.
struct PendingToolCall {
    id: String,
    name: String,
    json_buf: String,
}

/// Run the agent loop, processing user messages and streaming LLM responses.
///
/// This function runs until the user sends a Quit event or the channel closes.
/// It manages conversation history, streams LLM responses to the TUI, handles
/// tool calls through the approval engine, and loops back to the LLM when
/// tool results are available.
pub async fn run_agent_loop(
    client: Arc<dyn LlmClient>,
    registry: Registry,
    engine: Arc<ApprovalEngine>,
    model: String,
    max_tokens: u32,
    system_prompt: String,
    mut user_rx: mpsc::Receiver<UserEvent>,
    agent_tx: mpsc::Sender<AgentEvent>,
) {
    let mut messages: Vec<Message> = Vec::new();

    loop {
        // Wait for a user event.
        let event = match user_rx.recv().await {
            Some(e) => e,
            None => break, // Channel closed.
        };

        match event {
            UserEvent::Quit => break,
            UserEvent::Message(text) => {
                messages.push(Message::user(&text));

                // Enter the LLM conversation loop. After each round of tool calls,
                // we re-send the updated conversation to the LLM.
                if let Err(e) = conversation_turn(
                    &client,
                    &registry,
                    &engine,
                    &model,
                    max_tokens,
                    &system_prompt,
                    &mut messages,
                    &agent_tx,
                )
                .await
                {
                    let _ = agent_tx.send(AgentEvent::Error(e.to_string())).await;
                }

                let _ = agent_tx.send(AgentEvent::Done).await;
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
    system_prompt: &str,
    messages: &mut Vec<Message>,
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> anyhow::Result<()> {
    loop {
        let tool_defs = registry.to_definitions().await;

        let request = Request::new(model)
            .system(system_prompt)
            .max_tokens(max_tokens)
            .messages(messages.iter().cloned())
            .tools(tool_defs);

        let (assistant_blocks, stop_reason) =
            stream_response(client, &request, agent_tx).await?;

        // Record the assistant's response in conversation history.
        if !assistant_blocks.is_empty() {
            messages.push(Message {
                role: Role::Assistant,
                content: assistant_blocks.clone(),
            });
        }

        // If the LLM stopped because of tool use, execute tools and continue.
        if stop_reason == Some(StopReason::ToolUse) {
            let tool_results = execute_tool_calls(
                &assistant_blocks,
                registry,
                engine,
                agent_tx,
            )
            .await;

            if !tool_results.is_empty() {
                messages.push(Message::tool_results(tool_results));
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
                usage: _,
            } => {
                if let Some(reason) = sr {
                    stop_reason = Some(reason);
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
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> Vec<ContentBlock> {
    let mut results = Vec::new();

    for block in assistant_blocks {
        let (id, name, input) = match block {
            ContentBlock::ToolUse { id, name, input } => (id, name, input),
            _ => continue,
        };

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
                let decision = match tokio::time::timeout(Duration::from_secs(120), rx).await {
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
                        results
                            .push(ContentBlock::tool_error(id, "Denied by user".to_string()));
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
    if s.len() > 80 {
        format!("{}...", &s[..80])
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
}
