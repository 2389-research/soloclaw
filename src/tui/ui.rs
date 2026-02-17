// ABOUTME: Main TUI rendering function — assembles header, chat, input, and status bar.
// ABOUTME: Splits the terminal frame into vertical layout chunks and delegates to widgets.

use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::tui::state::TuiState;
use crate::tui::widgets::approval::approval_line;
use crate::tui::widgets::chat::render_chat_lines;
use crate::tui::widgets::status::status_line;

/// Render the full TUI screen layout to the given frame.
pub fn render(frame: &mut Frame, state: &TuiState) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(3),   // Chat area
            Constraint::Length(3), // Input area
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Header
    let header = Line::from(vec![
        Span::styled(
            " simpleclaw ",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} ", state.model),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} tools", state.tool_count),
            Style::default().fg(Color::White),
        ),
    ]);
    frame.render_widget(Paragraph::new(header), chunks[0]);

    // Chat area with optional approval prompt
    let mut chat_lines = render_chat_lines(&state.messages);

    if let Some(ref approval) = state.pending_approval {
        chat_lines.push(Line::from(""));
        let mut approval_lines = approval_line(&approval.description, approval.selected);
        chat_lines.append(&mut approval_lines);
    }

    let total_lines = chat_lines.len() as u16;
    let visible_height = chunks[1].height;
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Auto-scroll to bottom when scroll_offset is 0 (default), otherwise use user's offset
    let scroll = if state.scroll_offset == 0 {
        max_scroll
    } else {
        state.scroll_offset.min(max_scroll)
    };

    let chat = Paragraph::new(chat_lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(chat, chunks[1]);

    // Input area
    let input_block_style = if state.has_pending_approval() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(input_block_style)
        .title(" Input ");

    let input_text = if state.has_pending_approval() {
        "(approve/deny the tool call above)".to_string()
    } else if state.streaming {
        "(waiting for response...)".to_string()
    } else {
        state.input.clone()
    };

    let input_style = if state.has_pending_approval() || state.streaming {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let input = Paragraph::new(Span::styled(input_text, input_style)).block(input_block);
    frame.render_widget(input, chunks[2]);

    // Set cursor position when in normal input mode
    if !state.has_pending_approval() && !state.streaming {
        let cursor_x = chunks[2].x + 1 + state.cursor_pos as u16;
        let cursor_y = chunks[2].y + 1;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    // Status bar
    let status = status_line(
        &state.model,
        state.tool_count,
        state.total_tokens,
        state.streaming,
    );
    frame.render_widget(Paragraph::new(status), chunks[3]);
}
