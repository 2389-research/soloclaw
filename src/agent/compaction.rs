// ABOUTME: Conversation compaction — summarizes older messages when context limits approach.
// ABOUTME: Reduces conversation history using LLM summarization to stay within token budgets.

use std::sync::Arc;

use mux::prelude::*;

use crate::config::CompactionConfig;

pub const SUMMARY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process:";

/// Default token budget for retained user messages after compaction.
pub const DEFAULT_USER_MESSAGE_BUDGET_TOKENS: usize = 20_000;

/// Fraction of the context window that triggers automatic compaction.
const COMPACTION_THRESHOLD_RATIO: f64 = 0.9;

pub const SUMMARIZATION_PROMPT: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";

/// Heuristic token count: bytes / 4 (matching Codex strategy).
pub fn approx_token_count(text: &str) -> usize {
    text.len() / 4
}

/// Sum approximate token counts across all content blocks of all messages.
pub fn approx_messages_tokens(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|msg| &msg.content)
        .map(|block| match block {
            ContentBlock::Text { text } => approx_token_count(text),
            ContentBlock::ToolUse { input, .. } => approx_token_count(&input.to_string()),
            ContentBlock::ToolResult { content, .. } => approx_token_count(content),
        })
        .sum()
}

/// Calculate the token limit that triggers automatic compaction.
///
/// Default is 90% of context window, capped by an optional override.
pub fn auto_compact_limit(context_window: u64, override_limit: Option<u64>) -> u64 {
    let default_limit = (context_window as f64 * COMPACTION_THRESHOLD_RATIO) as u64;
    match override_limit {
        Some(cap) => default_limit.min(cap),
        None => default_limit,
    }
}

/// Return the known context window size for a given model identifier.
pub fn context_window_for_model(model: &str) -> u64 {
    if model.contains("claude") {
        200_000
    } else if model.contains("gpt-4o") || model.contains("gpt-5") {
        128_000
    } else if model.contains("gemini") {
        1_000_000
    } else {
        // Covers llama and other models; 128k is a safe default.
        128_000
    }
}

/// Check whether the current conversation exceeds the compaction threshold.
pub fn needs_compaction(messages: &[Message], model: &str, config: &CompactionConfig) -> bool {
    if !config.enabled {
        return false;
    }
    let context_window = context_window_for_model(model);
    let limit = auto_compact_limit(context_window, config.threshold_token_limit);
    let current_tokens = approx_messages_tokens(messages) as u64;
    current_tokens > limit
}

/// Extract text content from user messages, skipping summary messages.
pub fn collect_user_messages(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter(|msg| matches!(msg.role, Role::User))
        .filter_map(|msg| {
            let texts: Vec<&str> = msg
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();

            if texts.is_empty() {
                return None;
            }

            let combined = texts.join("\n");
            if combined.starts_with(SUMMARY_PREFIX) {
                return None;
            }

            Some(combined)
        })
        .collect()
}

/// Build compacted conversation history from a summary and recent user messages.
///
/// Selects user messages backward from the most recent, within the given token budget.
/// If a message exceeds the remaining budget, it is truncated with a marker.
/// Returns messages in chronological order: selected user messages, then summary message.
pub fn build_compacted_history(
    user_messages: &[String],
    summary_text: &str,
    max_user_tokens: usize,
) -> Vec<Message> {
    let mut selected: Vec<Message> = Vec::new();
    let mut remaining_budget = max_user_tokens;

    // Walk backward through user messages, selecting within budget.
    for text in user_messages.iter().rev() {
        let tokens = approx_token_count(text);
        if tokens <= remaining_budget {
            selected.push(Message::user(text.clone()));
            remaining_budget -= tokens;
        } else if remaining_budget > 0 {
            // Truncate this message to fit within remaining budget.
            let char_limit = remaining_budget * 4;
            let truncated: String = text.chars().take(char_limit).collect();
            let omitted = tokens.saturating_sub(remaining_budget);
            let truncated_msg = format!("{}...{} tokens truncated...", truncated, omitted);
            selected.push(Message::user(truncated_msg));
            remaining_budget = 0;
        }
        // If remaining_budget is 0, skip older messages.
    }

    // Reverse to restore chronological order.
    selected.reverse();

    // Append the summary as a user message with the SUMMARY_PREFIX.
    let summary_content = format!("{}\n\n{}", SUMMARY_PREFIX, summary_text);
    selected.push(Message::user(summary_content));

    selected
}

/// Run compaction: send the full conversation to the LLM with a summarization prompt
/// and return the summary text.
pub async fn run_compaction(
    client: &Arc<dyn LlmClient>,
    model: &str,
    max_tokens: u32,
    messages: &[Message],
) -> anyhow::Result<String> {
    // Build a request with the full conversation plus the summarization prompt.
    let mut compaction_messages: Vec<Message> = messages.to_vec();
    compaction_messages.push(Message::user(SUMMARIZATION_PROMPT));

    let request = Request::new(model)
        .max_tokens(max_tokens)
        .messages(compaction_messages);

    let response = client.create_message(&request).await?;
    Ok(response.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approx_token_count_returns_reasonable_values() {
        // "hello world" is 11 bytes, so 11/4 = 2
        assert_eq!(approx_token_count("hello world"), 2);
        // Empty string
        assert_eq!(approx_token_count(""), 0);
        // 100 chars = 25 tokens
        let text = "a".repeat(100);
        assert_eq!(approx_token_count(&text), 25);
    }

    #[test]
    fn approx_messages_tokens_sums_across_blocks() {
        let messages = vec![
            Message::user("hello"), // 5 bytes = 1 token
            Message::assistant("world of code"), // 13 bytes = 3 tokens
        ];
        let total = approx_messages_tokens(&messages);
        assert_eq!(total, 4);
    }

    #[test]
    fn auto_compact_limit_calculates_90_percent() {
        let limit = auto_compact_limit(200_000, None);
        assert_eq!(limit, 180_000);
    }

    #[test]
    fn auto_compact_limit_with_override_caps() {
        // Override is lower than 90%
        let limit = auto_compact_limit(200_000, Some(100_000));
        assert_eq!(limit, 100_000);

        // Override is higher than 90% — still uses 90%
        let limit = auto_compact_limit(200_000, Some(190_000));
        assert_eq!(limit, 180_000);
    }

    #[test]
    fn context_window_for_known_models() {
        assert_eq!(context_window_for_model("claude-sonnet-4-5-20250929"), 200_000);
        assert_eq!(context_window_for_model("claude-3-opus"), 200_000);
        assert_eq!(context_window_for_model("gpt-4o-mini"), 128_000);
        assert_eq!(context_window_for_model("gpt-5"), 128_000);
        assert_eq!(context_window_for_model("gemini-2.5-pro"), 1_000_000);
        assert_eq!(context_window_for_model("llama3.2"), 128_000);
        assert_eq!(context_window_for_model("unknown-model"), 128_000);
    }

    #[test]
    fn needs_compaction_false_for_small_conversations() {
        let messages = vec![Message::user("hello"), Message::assistant("hi there")];
        let config = CompactionConfig::default();
        assert!(!needs_compaction(&messages, "claude-sonnet-4-5-20250929", &config));
    }

    #[test]
    fn needs_compaction_true_when_over_threshold() {
        // Create a message that exceeds 90% of 200k = 180k tokens = 720k bytes
        let big_text = "x".repeat(800_000);
        let messages = vec![Message::user(big_text)];
        let config = CompactionConfig::default();
        assert!(needs_compaction(&messages, "claude-sonnet-4-5-20250929", &config));
    }

    #[test]
    fn needs_compaction_false_when_disabled() {
        let big_text = "x".repeat(800_000);
        let messages = vec![Message::user(big_text)];
        let config = CompactionConfig {
            enabled: false,
            ..Default::default()
        };
        assert!(!needs_compaction(&messages, "claude-sonnet-4-5-20250929", &config));
    }

    #[test]
    fn collect_user_messages_filters_only_user_text() {
        let messages = vec![
            Message::user("first question"),
            Message::assistant("first answer"),
            Message::user("second question"),
            Message::assistant("second answer"),
        ];
        let user_msgs = collect_user_messages(&messages);
        assert_eq!(user_msgs, vec!["first question", "second question"]);
    }

    #[test]
    fn collect_user_messages_skips_summaries() {
        let summary = format!("{}\n\nHere is a summary...", SUMMARY_PREFIX);
        let messages = vec![
            Message::user("real question"),
            Message::user(summary),
            Message::user("another question"),
        ];
        let user_msgs = collect_user_messages(&messages);
        assert_eq!(user_msgs, vec!["real question", "another question"]);
    }

    #[test]
    fn collect_user_messages_skips_tool_result_only_messages() {
        let messages = vec![
            Message::user("question"),
            Message::tool_results(vec![ContentBlock::tool_result("id1", "output")]),
        ];
        let user_msgs = collect_user_messages(&messages);
        assert_eq!(user_msgs, vec!["question"]);
    }

    #[test]
    fn build_compacted_history_selects_messages_backward() {
        let user_messages = vec![
            "old message".to_string(),
            "middle message".to_string(),
            "recent message".to_string(),
        ];
        // Budget of 10 tokens = 40 bytes. "recent message" = 14 bytes = 3 tokens,
        // "middle message" = 14 bytes = 3 tokens, "old message" = 11 bytes = 2 tokens.
        // Total = 8 tokens, fits in budget.
        let result = build_compacted_history(&user_messages, "summary", 10);

        // Should have all 3 user messages + 1 summary = 4 messages.
        assert_eq!(result.len(), 4);

        // Last message should be the summary.
        if let ContentBlock::Text { text } = &result[3].content[0] {
            assert!(text.starts_with(SUMMARY_PREFIX));
            assert!(text.contains("summary"));
        } else {
            panic!("expected text block in summary message");
        }
    }

    #[test]
    fn build_compacted_history_truncates_overflow() {
        let user_messages = vec![
            "x".repeat(200), // 50 tokens
            "y".repeat(40),  // 10 tokens
        ];
        // Budget = 15 tokens. "y" (10 tokens) fits. "x" (50 tokens) has 5 token budget remaining.
        let result = build_compacted_history(&user_messages, "summary text", 15);

        // Should have: truncated "x" message, "y" message, summary = 3 messages.
        assert_eq!(result.len(), 3);

        // First message should be truncated.
        if let ContentBlock::Text { text } = &result[0].content[0] {
            assert!(text.contains("tokens truncated"));
        } else {
            panic!("expected text block with truncation marker");
        }
    }

    #[test]
    fn build_compacted_history_appends_summary_with_prefix() {
        let user_messages = vec!["question".to_string()];
        let result = build_compacted_history(&user_messages, "my summary", 100);

        // Last message is the summary.
        let last = result.last().unwrap();
        assert!(matches!(last.role, Role::User));
        if let ContentBlock::Text { text } = &last.content[0] {
            assert!(text.starts_with(SUMMARY_PREFIX));
            assert!(text.contains("my summary"));
        } else {
            panic!("expected text block in summary message");
        }
    }
}
