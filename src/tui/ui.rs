// ABOUTME: Main TUI rendering function — assembles header, chat, input, and status bar.
// ABOUTME: Splits the terminal frame into vertical layout chunks and delegates to widgets.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::state::TuiState;
use crate::tui::widgets::approval::approval_line;
use crate::tui::widgets::chat::render_chat_lines;
use crate::tui::widgets::status::status_line;

/// Render the full TUI screen layout to the given frame.
pub fn render(frame: &mut Frame, state: &mut TuiState) {
    let area = frame.area();

    let has_approval = state.has_pending_approval();

    // Dynamic layout: insert a dedicated approval area when one is pending.
    let constraints = if has_approval {
        vec![
            Constraint::Length(1), // Header
            Constraint::Min(3),    // Chat area
            Constraint::Length(3), // Approval prompt (description + options + blank)
            Constraint::Length(3), // Input area
            Constraint::Length(1), // Status bar
        ]
    } else {
        vec![
            Constraint::Length(1), // Header
            Constraint::Min(3),    // Chat area
            Constraint::Length(3), // Input area
            Constraint::Length(1), // Status bar
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Header
    let header = Line::from(Span::styled(
        " soloclaw",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(header), chunks[0]);

    // Chat area (messages only, no approval)
    let chat_lines = render_chat_lines(&state.messages);

    let chat_chunk = chunks[1];
    let visible_height = chat_chunk.height;
    let total_lines = wrapped_line_count(&chat_lines, chat_chunk.width);
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Cap scroll_offset so it can't go past the top of the content.
    if state.scroll_offset > max_scroll {
        state.scroll_offset = max_scroll;
    }

    // scroll_offset is lines scrolled up from the bottom (0 = at bottom)
    let scroll = max_scroll.saturating_sub(state.scroll_offset);

    frame.render_widget(
        Paragraph::new(chat_lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        chat_chunk,
    );

    // Approval area (only when pending)
    let (input_chunk, status_chunk) = if has_approval {
        if let Some(ref approval) = state.pending_approval {
            let approval_lines = approval_line(&approval.description, approval.selected);
            let approval_widget = Paragraph::new(approval_lines);
            frame.render_widget(approval_widget, chunks[2]);
        }
        (chunks[3], chunks[4])
    } else {
        (chunks[2], chunks[3])
    };

    // Input area
    let input_block_style = if has_approval {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let input_block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(input_block_style);

    let input_text = if has_approval {
        "(approve/deny the tool call above)".to_string()
    } else if state.streaming {
        "(waiting for response...)".to_string()
    } else {
        state.input.clone()
    };

    let input_style = if has_approval || state.streaming {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let input = Paragraph::new(Span::styled(input_text, input_style)).block(input_block);
    frame.render_widget(input, input_chunk);

    // Set cursor position when in normal input mode
    if !has_approval && !state.streaming && input_chunk.width > 0 && input_chunk.height > 1 {
        state.clamp_cursor();

        let cursor_byte_index = state.cursor_byte_index();
        let visual_col = UnicodeWidthStr::width(&state.input[..cursor_byte_index]);
        let max_visual_col = input_chunk.width.saturating_sub(1) as usize;
        let clamped_visual_col = visual_col.min(max_visual_col);

        let cursor_x = input_chunk.x.saturating_add(clamped_visual_col as u16);
        let cursor_y = input_chunk.y.saturating_add(1);
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    // Status bar
    let status = status_line(
        &state.model,
        state.tool_count,
        state.total_tokens,
        state.streaming,
    );
    frame.render_widget(Paragraph::new(status), status_chunk);
}

fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> u16 {
    if width == 0 {
        return 0;
    }

    let max_width = width as usize;
    let mut total = 0u16;

    for line in lines {
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        total = total.saturating_add(wrap_rows_for_text(&text, max_width) as u16);
    }

    total
}

fn wrap_rows_for_text(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }

    let mut rows = 1usize;
    let mut col = 0usize;
    let mut token = String::new();
    let mut in_whitespace = None;

    for ch in text.chars() {
        let is_ws = ch.is_whitespace();
        match in_whitespace {
            Some(current) if current == is_ws => token.push(ch),
            Some(_) => {
                apply_wrap_token(
                    &token,
                    in_whitespace.unwrap_or(false),
                    width,
                    &mut rows,
                    &mut col,
                );
                token.clear();
                token.push(ch);
                in_whitespace = Some(is_ws);
            }
            None => {
                token.push(ch);
                in_whitespace = Some(is_ws);
            }
        }
    }

    if !token.is_empty() {
        apply_wrap_token(
            &token,
            in_whitespace.unwrap_or(false),
            width,
            &mut rows,
            &mut col,
        );
    }

    rows
}

fn apply_wrap_token(
    token: &str,
    is_whitespace: bool,
    width: usize,
    rows: &mut usize,
    col: &mut usize,
) {
    let len = token.chars().map(display_width).sum::<usize>();

    if len > width {
        if !is_whitespace && *col > 0 {
            *rows += 1;
            *col = 0;
        }
        for ch in token.chars() {
            let ch_width = display_width(ch);
            if ch_width == 0 {
                continue;
            }
            if *col + ch_width > width {
                *rows += 1;
                *col = 0;
            }
            *col += ch_width;
        }
        return;
    }

    if *col + len > width {
        *rows += 1;
        *col = 0;
    }

    *col += len;
}

fn display_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}
