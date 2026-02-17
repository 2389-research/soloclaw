// ABOUTME: Status bar widget — renders directory, context usage bar, and elapsed session time.
// ABOUTME: Displayed at the bottom of the TUI as a single-line summary.

use std::time::Instant;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Parameters for rendering the status bar.
pub struct StatusBarParams<'a> {
    pub workspace_dir: &'a str,
    pub context_used: u64,
    pub context_window: u64,
    pub session_start: Instant,
    pub streaming: bool,
}

/// Render the status bar: directory │ context bar percentage │ elapsed time.
pub fn status_line(params: &StatusBarParams) -> Line<'static> {
    let dim = Style::default().fg(Color::DarkGray);

    // Directory name (last component of path).
    let dir_name = params
        .workspace_dir
        .rsplit('/')
        .next()
        .unwrap_or(params.workspace_dir);

    let context_pct = if params.context_window > 0 {
        ((params.context_used as f64 / params.context_window as f64) * 100.0).min(100.0)
    } else {
        0.0
    };

    let bar = render_context_bar(context_pct, 12);

    let bar_color = if context_pct >= 90.0 {
        Color::Red
    } else if context_pct >= 70.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let elapsed = format_elapsed(params.session_start);

    let mut spans = vec![
        Span::styled(" \u{1F4C1} ", dim),
        Span::styled(
            format!("{} ", dir_name),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("\u{2502} ", dim),
        Span::styled(bar, Style::default().fg(bar_color)),
        Span::styled(
            format!(" {:.0}% ", context_pct),
            Style::default().fg(Color::White),
        ),
        Span::styled("\u{2502} ", dim),
        Span::styled(
            format!("\u{23F1} {} ", elapsed),
            Style::default().fg(Color::White),
        ),
    ];

    if params.streaming {
        spans.push(Span::styled("\u{2502} ", dim));
        spans.push(Span::styled(
            "streaming... ",
            Style::default().fg(Color::Yellow),
        ));
    }

    Line::from(spans)
}

/// Render a context usage bar using block characters.
/// `filled_chars` total width, proportional fill with block elements.
fn render_context_bar(pct: f64, width: usize) -> String {
    let filled = (pct / 100.0) * width as f64;
    let full_blocks = filled.floor() as usize;
    let remainder = filled - filled.floor();

    let mut bar = String::with_capacity(width * 4);
    for _ in 0..full_blocks.min(width) {
        bar.push('\u{2588}'); // Full block
    }

    if full_blocks < width {
        // Use braille-style partial fill for the fractional block.
        let partial = if remainder >= 0.5 {
            '\u{2593}' // Dark shade
        } else if remainder >= 0.25 {
            '\u{2592}' // Medium shade
        } else {
            '\u{2591}' // Light shade
        };
        bar.push(partial);

        // Fill remaining with light dots.
        for _ in (full_blocks + 1)..width {
            bar.push('\u{2591}');
        }
    }

    bar
}

/// Format elapsed time as human-readable "Xh Ym" or "Xm Ys".
fn format_elapsed(start: Instant) -> String {
    let secs = start.elapsed().as_secs();
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;

    if hours > 0 {
        format!("{}h {:02}m", hours, mins)
    } else {
        let s = secs % 60;
        format!("{}m {:02}s", mins, s)
    }
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
        let params = StatusBarParams {
            workspace_dir: "/home/user/my-project",
            context_used: 120_000,
            context_window: 200_000,
            session_start: Instant::now(),
            streaming: true,
        };
        let line = status_line(&params);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("my-project"));
        assert!(text.contains("60%"));
        assert!(text.contains("streaming..."));
    }

    #[test]
    fn status_line_no_streaming() {
        let params = StatusBarParams {
            workspace_dir: "/tmp/test-dir",
            context_used: 0,
            context_window: 128_000,
            session_start: Instant::now(),
            streaming: false,
        };
        let line = status_line(&params);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("test-dir"));
        assert!(text.contains("0%"));
        assert!(!text.contains("streaming"));
    }

    #[test]
    fn context_bar_empty() {
        let bar = render_context_bar(0.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(!bar.contains('\u{2588}')); // No full blocks
    }

    #[test]
    fn context_bar_full() {
        let bar = render_context_bar(100.0, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.chars().all(|c| c == '\u{2588}'));
    }

    #[test]
    fn context_bar_half() {
        let bar = render_context_bar(50.0, 10);
        assert_eq!(bar.chars().count(), 10);
        let full_count = bar.chars().filter(|&c| c == '\u{2588}').count();
        assert_eq!(full_count, 5);
    }

    #[test]
    fn format_elapsed_minutes() {
        // Can't easily test with Instant, so test the formatting logic indirectly
        // by checking the status line contains the timer emoji.
        let params = StatusBarParams {
            workspace_dir: "/tmp/test",
            context_used: 0,
            context_window: 100_000,
            session_start: Instant::now(),
            streaming: false,
        };
        let line = status_line(&params);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("0m"));
    }

    #[test]
    fn context_percentage_capped_at_100() {
        let params = StatusBarParams {
            workspace_dir: "/tmp",
            context_used: 300_000,
            context_window: 200_000,
            session_start: Instant::now(),
            streaming: false,
        };
        let line = status_line(&params);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("100%"));
    }
}
