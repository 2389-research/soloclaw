// ABOUTME: Status bar widget — renders model name, tool count, token usage, and streaming indicator.
// ABOUTME: Displayed at the bottom of the TUI as a single-line summary.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Render the status bar line with model, tools, tokens, and streaming state.
pub fn status_line(
    model: &str,
    tool_count: usize,
    total_tokens: u64,
    streaming: bool,
) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);
    let mut spans = vec![
        Span::styled(format!(" {} ", model), Style::default().fg(Color::Cyan)),
        Span::styled("| ", dim),
        Span::styled(
            format!("{} tools ", tool_count),
            Style::default().fg(Color::White),
        ),
        Span::styled("| ", dim),
        Span::styled(
            format!("{} tokens ", format_tokens(total_tokens)),
            Style::default().fg(Color::White),
        ),
    ];

    if streaming {
        spans.push(Span::styled("| ", dim));
        spans.push(Span::styled(
            "streaming... ",
            Style::default().fg(Color::Yellow),
        ));
    }

    Line::from(spans)
}

/// Format a token count for display: small numbers as-is, thousands as X.Xk, millions as X.XM.
pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(42), "42");
        assert_eq!(format_tokens(999), "999");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(1_000), "1.0k");
        assert_eq!(format_tokens(1_500), "1.5k");
        assert_eq!(format_tokens(12_345), "12.3k");
        assert_eq!(format_tokens(999_999), "1000.0k");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(1_000_000), "1.0M");
        assert_eq!(format_tokens(2_500_000), "2.5M");
        assert_eq!(format_tokens(10_000_000), "10.0M");
    }

    #[test]
    fn status_line_shows_streaming() {
        let line = status_line("claude-sonnet", 5, 1500, true);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("claude-sonnet"));
        assert!(text.contains("5 tools"));
        assert!(text.contains("1.5k tokens"));
        assert!(text.contains("streaming..."));
    }

    #[test]
    fn status_line_no_streaming() {
        let line = status_line("gpt-4", 3, 500, false);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("gpt-4"));
        assert!(!text.contains("streaming"));
    }
}
