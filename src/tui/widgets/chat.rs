// ABOUTME: Chat widget — renders chat messages into styled ratatui Lines.
// ABOUTME: Each message kind (user, assistant, tool, system) has distinct visual styling.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::state::{ChatMessage, ChatMessageKind, ToolCallStatus};

/// Render a slice of chat messages into styled Lines for display.
pub fn render_chat_lines(messages: &[ChatMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        // Add a blank separator line between message groups.
        // ToolResult is part of the preceding ToolCall group, so no separator before it.
        if idx > 0 && !matches!(msg.kind, ChatMessageKind::ToolResult { .. }) {
            lines.push(Line::from(""));
        }

        match &msg.kind {
            ChatMessageKind::User => {
                lines.push(Line::from(vec![
                    Span::styled(
                        "❯ ",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(msg.content.clone()),
                ]));
            }
            ChatMessageKind::Assistant => {
                // First line gets the prefix, subsequent lines are plain.
                let content_lines: Vec<&str> = msg.content.split('\n').collect();
                for (i, text) in content_lines.iter().enumerate() {
                    if i == 0 {
                        lines.push(Line::from(vec![
                            Span::styled(
                                "⏺ ",
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(text.to_string()),
                        ]));
                    } else {
                        lines.push(Line::from(Span::raw(text.to_string())));
                    }
                }
            }
            ChatMessageKind::ToolCall { tool_name, status } => {
                let status_str = match status {
                    ToolCallStatus::Allowed => "[allowed]",
                    ToolCallStatus::Denied => "[denied]",
                    ToolCallStatus::Pending => "[pending]",
                    ToolCallStatus::TimedOut => "[timed out]",
                };
                lines.push(Line::from(Span::styled(
                    format!("⚙ {}({}) {}", tool_name, msg.content, status_str),
                    Style::default().fg(Color::Yellow),
                )));
            }
            ChatMessageKind::ToolResult { is_error } => {
                let style = if *is_error {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let content_lines: Vec<&str> = msg.content.split('\n').collect();
                let max_lines = 10;
                let truncated = content_lines.len() > max_lines;
                for text in content_lines.iter().take(max_lines) {
                    lines.push(Line::from(Span::styled(format!("   {}", text), style)));
                }
                if truncated {
                    lines.push(Line::from(Span::styled(
                        format!("   ... ({} more lines)", content_lines.len() - max_lines),
                        style,
                    )));
                }
            }
            ChatMessageKind::System => {
                lines.push(Line::from(Span::styled(
                    format!("[system] {}", msg.content),
                    Style::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
        }
    }

    lines
}

/// Create a scrollable Paragraph widget from chat messages.
pub fn chat_widget(messages: &[ChatMessage], scroll_offset: u16) -> Paragraph<'static> {
    let lines = render_chat_lines(messages);
    Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_has_green_prefix() {
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::User,
            content: "hello".to_string(),
        }];
        let lines = render_chat_lines(&messages);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert!(spans.len() >= 2);
        assert_eq!(spans[0].content, "❯ ");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn assistant_message_has_cyan_prefix() {
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::Assistant,
            content: "hi there".to_string(),
        }];
        let lines = render_chat_lines(&messages);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "⏺ ");
        assert_eq!(spans[0].style.fg, Some(Color::Cyan));
    }

    #[test]
    fn multiline_assistant_message() {
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::Assistant,
            content: "line1\nline2\nline3".to_string(),
        }];
        let lines = render_chat_lines(&messages);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn tool_call_has_gear_prefix() {
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::ToolCall {
                tool_name: "bash".to_string(),
                status: ToolCallStatus::Allowed,
            },
            content: "ls -la".to_string(),
        }];
        let lines = render_chat_lines(&messages);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].style.fg, Some(Color::Yellow));
        assert!(spans[0].content.contains("⚙"));
        assert!(spans[0].content.contains("bash"));
        assert!(spans[0].content.contains("[allowed]"));
    }

    #[test]
    fn tool_result_truncates_long_output() {
        let long_content = (0..15)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::ToolResult { is_error: false },
            content: long_content,
        }];
        let lines = render_chat_lines(&messages);
        // 10 visible lines + 1 truncation indicator
        assert_eq!(lines.len(), 11);
        let last_line = &lines[10].spans[0].content;
        assert!(last_line.contains("5 more lines"));
    }

    #[test]
    fn system_message_is_italic_gray() {
        let messages = vec![ChatMessage {
            kind: ChatMessageKind::System,
            content: "connected".to_string(),
        }];
        let lines = render_chat_lines(&messages);
        assert_eq!(lines.len(), 1);
        let spans = &lines[0].spans;
        assert_eq!(spans[0].style.fg, Some(Color::DarkGray));
        assert!(spans[0].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn blank_separator_between_message_groups() {
        let messages = vec![
            ChatMessage {
                kind: ChatMessageKind::User,
                content: "hi".to_string(),
            },
            ChatMessage {
                kind: ChatMessageKind::Assistant,
                content: "hello".to_string(),
            },
        ];
        let lines = render_chat_lines(&messages);
        // user line, blank separator, assistant line
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].spans.len(), 0);
    }

    #[test]
    fn no_separator_between_tool_call_and_result() {
        let messages = vec![
            ChatMessage {
                kind: ChatMessageKind::ToolCall {
                    tool_name: "bash".to_string(),
                    status: ToolCallStatus::Allowed,
                },
                content: "ls".to_string(),
            },
            ChatMessage {
                kind: ChatMessageKind::ToolResult { is_error: false },
                content: "file.txt".to_string(),
            },
        ];
        let lines = render_chat_lines(&messages);
        // tool call line, tool result line (no separator)
        assert_eq!(lines.len(), 2);
    }
}
