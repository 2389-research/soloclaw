// ABOUTME: Question prompt widget — inline TUI prompt for LLM-initiated questions.
// ABOUTME: Shows the question text with a distinctive Cyan header and usage hint.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Render the question prompt as two Lines: header with question + usage hint.
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
}
