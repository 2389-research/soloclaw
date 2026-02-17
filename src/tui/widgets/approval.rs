// ABOUTME: Approval prompt widget — inline TUI prompt for tool call approval.
// ABOUTME: Shows description and three selectable options: Allow Once, Always Allow, Deny.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// The three approval options presented to the user.
pub const APPROVAL_OPTIONS: &[&str] = &["Allow Once", "Always Allow", "Deny"];

/// Render the approval prompt as two Lines: description + selectable options.
pub fn approval_line(description: &str, selected: usize) -> Vec<Line<'static>> {
    let header = Line::from(vec![
        Span::styled(
            "APPROVE? ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(description.to_string(), Style::default().fg(Color::White)),
    ]);

    let mut option_spans = Vec::new();
    for (i, option) in APPROVAL_OPTIONS.iter().enumerate() {
        if i > 0 {
            option_spans.push(Span::raw("  "));
        }

        let label = format!(" [{}] {} ", i + 1, option);
        if i == selected {
            option_spans.push(Span::styled(
                label,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            option_spans.push(Span::styled(label, Style::default().fg(Color::DarkGray)));
        }
    }

    let options = Line::from(option_spans);

    vec![header, options]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_line_has_all_options() {
        let lines = approval_line("run bash command", 0);
        assert_eq!(lines.len(), 2);

        // Header line should contain APPROVE? and description
        let header_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(header_text.contains("APPROVE?"));
        assert!(header_text.contains("run bash command"));

        // Options line should have all three options
        let options_text: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(options_text.contains("Allow Once"));
        assert!(options_text.contains("Always Allow"));
        assert!(options_text.contains("Deny"));
    }

    #[test]
    fn selected_index_is_valid() {
        // Test each valid selection index renders without panic
        for i in 0..APPROVAL_OPTIONS.len() {
            let lines = approval_line("test", i);
            assert_eq!(lines.len(), 2);
        }

        // Verify the selected option is highlighted (black on yellow)
        let lines = approval_line("test", 1);
        let option_spans = &lines[1].spans;
        // Find the span for "Always Allow" (the selected one)
        let selected_span = option_spans
            .iter()
            .find(|s| s.content.contains("Always Allow"))
            .expect("should have Always Allow span");
        assert_eq!(selected_span.style.fg, Some(Color::Black));
        assert_eq!(selected_span.style.bg, Some(Color::Yellow));
    }
}
