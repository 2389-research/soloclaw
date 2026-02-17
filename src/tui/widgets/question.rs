// ABOUTME: Question prompt widget — inline TUI prompt for LLM-initiated questions.
// ABOUTME: Supports both multiple-choice (horizontal options) and free-text question modes.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render a free-text question prompt as two Lines: header with question + usage hint.
pub fn question_lines(question: &str) -> Vec<Line<'static>> {
    let header = Line::from(vec![
        Span::styled(
            "QUESTION: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(question.to_string(), Style::default().fg(Color::White)),
    ]);

    let hint = Line::from(Span::styled(
        "(Type your answer and press Enter, or Esc to skip)",
        Style::default().fg(Color::DarkGray),
    ));

    vec![header, hint]
}

/// Render a multiple-choice question prompt: header + options line + hint.
pub fn multichoice_lines(question: &str, options: &[String], selected: usize) -> Vec<Line<'static>> {
    let header = Line::from(vec![
        Span::styled(
            "QUESTION: ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(question.to_string(), Style::default().fg(Color::White)),
    ]);

    let mut option_spans: Vec<Span<'static>> = Vec::new();
    for (i, opt) in options.iter().enumerate() {
        if i > 0 {
            option_spans.push(Span::raw("  "));
        }
        let label = format!("[{}] {}", i + 1, opt);
        if i == selected {
            option_spans.push(Span::styled(
                label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
        } else {
            option_spans.push(Span::styled(
                label,
                Style::default().fg(Color::White),
            ));
        }
    }
    let options_line = Line::from(option_spans);

    let hint = Line::from(Span::styled(
        "(Left/Right to navigate, Enter or number key to select, Esc to skip)",
        Style::default().fg(Color::DarkGray),
    ));

    vec![header, options_line, hint]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_lines_has_two_lines() {
        let lines = question_lines("What is your name?");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn header_contains_question_prefix_and_text() {
        let lines = question_lines("What color do you prefer?");
        let header_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(header_text.contains("QUESTION:"));
        assert!(header_text.contains("What color do you prefer?"));
    }

    #[test]
    fn header_uses_cyan_color() {
        let lines = question_lines("test");
        let question_label = &lines[0].spans[0];
        assert_eq!(question_label.style.fg, Some(Color::Cyan));
    }

    #[test]
    fn hint_line_mentions_enter_and_esc() {
        let lines = question_lines("test");
        let hint_text: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(hint_text.contains("Enter"));
        assert!(hint_text.contains("Esc"));
    }

    #[test]
    fn empty_question_still_renders() {
        let lines = question_lines("");
        assert_eq!(lines.len(), 2);
        let header_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(header_text.contains("QUESTION:"));
    }

    // --- Multiple choice tests ---

    #[test]
    fn multichoice_has_three_lines() {
        let options = vec!["red".to_string(), "green".to_string(), "blue".to_string()];
        let lines = multichoice_lines("Pick a color", &options, 0);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn multichoice_header_contains_question() {
        let options = vec!["yes".to_string(), "no".to_string()];
        let lines = multichoice_lines("Continue?", &options, 0);
        let header_text: String = lines[0]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(header_text.contains("QUESTION:"));
        assert!(header_text.contains("Continue?"));
    }

    #[test]
    fn multichoice_options_show_numbered() {
        let options = vec!["red".to_string(), "green".to_string()];
        let lines = multichoice_lines("Pick", &options, 0);
        let options_text: String = lines[1]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(options_text.contains("[1] red"));
        assert!(options_text.contains("[2] green"));
    }

    #[test]
    fn multichoice_selected_has_reversed_style() {
        let options = vec!["a".to_string(), "b".to_string()];
        let lines = multichoice_lines("Pick", &options, 1);
        // Find the selected option span (should have REVERSED modifier)
        let selected_span = lines[1]
            .spans
            .iter()
            .find(|s| s.content.contains("[2] b"))
            .expect("should find selected option");
        assert!(selected_span.style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn multichoice_hint_mentions_navigation() {
        let options = vec!["x".to_string()];
        let lines = multichoice_lines("Pick", &options, 0);
        let hint_text: String = lines[2]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(hint_text.contains("Left/Right"));
        assert!(hint_text.contains("Enter"));
        assert!(hint_text.contains("Esc"));
    }
}
