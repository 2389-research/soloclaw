// ABOUTME: Main TUI rendering function — assembles header, chat, input, and status bar.
// ABOUTME: Splits the terminal frame into vertical layout chunks and delegates to widgets.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::state::TuiState;
use crate::tui::widgets::approval::approval_line;
use crate::tui::widgets::chat::render_chat_lines;
use crate::tui::widgets::question::question_lines;
use crate::tui::widgets::status::{StatusBarParams, status_line};

/// Render the full TUI screen layout to the given frame.
pub fn render(frame: &mut Frame, state: &mut TuiState) {
    let area = frame.area();

    let has_approval = state.has_pending_approval();
    let has_question = state.has_pending_question();

    // Maximum height the input area can grow to (in terminal rows).
    const MAX_INPUT_HEIGHT: u16 = 8;

    // Calculate input height based on line count; fixed when approval is pending.
    // Streaming mode now allows typing, so it uses dynamic height too.
    let input_height = if has_approval {
        3 // fixed height when approval pending
    } else {
        // +2 accounts for top and bottom borders
        (state.input_line_count() as u16 + 2).clamp(3, MAX_INPUT_HEIGHT)
    };

    // Dynamic layout: insert a dedicated prompt area when approval or question is pending.
    let constraints = if has_approval || has_question {
        vec![
            Constraint::Length(1),            // Header
            Constraint::Min(3),               // Chat area
            Constraint::Length(3),            // Approval/question prompt area
            Constraint::Length(input_height), // Input area
            Constraint::Length(1),            // Status bar
        ]
    } else {
        vec![
            Constraint::Length(1),            // Header
            Constraint::Min(3),               // Chat area
            Constraint::Length(input_height), // Input area
            Constraint::Length(1),            // Status bar
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

    // Use ratatui's own line_count() to get an accurate wrapped line count
    // that exactly matches its internal rendering. This prevents scroll
    // miscalculations that could hide the bottom of chat content.
    let chat_paragraph = Paragraph::new(chat_lines.clone())
        .wrap(Wrap { trim: false });
    let total_lines = chat_paragraph.line_count(chat_chunk.width) as u16;
    let max_scroll = total_lines.saturating_sub(visible_height);

    // Cap scroll_offset so it can't go past the top of the content.
    if state.scroll_offset > max_scroll {
        state.scroll_offset = max_scroll;
    }

    // scroll_offset is lines scrolled up from the bottom (0 = at bottom)
    let scroll = max_scroll.saturating_sub(state.scroll_offset);

    frame.render_widget(
        chat_paragraph.scroll((scroll, 0)),
        chat_chunk,
    );

    // Approval or question area (only when pending)
    let (input_chunk, status_chunk) = if has_approval {
        if let Some(ref approval) = state.pending_approval {
            let approval_lines = approval_line(&approval.description, approval.selected);
            let approval_widget = Paragraph::new(approval_lines);
            frame.render_widget(approval_widget, chunks[2]);
        }
        (chunks[3], chunks[4])
    } else if has_question {
        if let Some(ref question) = state.pending_question {
            let q_lines = question_lines(&question.question);
            let question_widget = Paragraph::new(q_lines);
            frame.render_widget(question_widget, chunks[2]);
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

    let mut input_block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_style(input_block_style);

    // Show streaming/queued indicator in the input border title.
    if state.streaming {
        let title = if state.queued_message.is_some() {
            " message queued "
        } else {
            " streaming... "
        };
        input_block = input_block.title(Span::styled(
            title,
            Style::default().fg(Color::DarkGray),
        ));
    }

    let input_text = if has_approval {
        "(approve/deny the tool call above)".to_string()
    } else {
        // Normal input, question mode, and streaming all show the input buffer.
        state.input.clone()
    };

    let input_style = if has_approval {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let input = Paragraph::new(Span::styled(input_text, input_style)).block(input_block);
    frame.render_widget(input, input_chunk);

    // Set cursor position when editing: normal input, question mode, or streaming.
    if !has_approval && input_chunk.width > 0 && input_chunk.height > 1 {
        state.clamp_cursor();

        let cursor_line = state.cursor_line();
        let cursor_col = state.cursor_column();

        // Compute the visual (display) width of the text before the cursor on its line.
        let lines = state.input_lines();
        let line_text = lines.get(cursor_line).unwrap_or(&"");
        let prefix: String = line_text.chars().take(cursor_col).collect();
        let visual_col = UnicodeWidthStr::width(prefix.as_str());

        let max_visual_col = input_chunk.width.saturating_sub(1) as usize;
        let clamped_visual_col = visual_col.min(max_visual_col);

        let cursor_x = input_chunk.x.saturating_add(clamped_visual_col as u16);
        // +1 for the top border, then offset by the cursor's line index.
        let cursor_y = input_chunk.y.saturating_add(1 + cursor_line as u16);
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    // Status bar
    let status = status_line(&StatusBarParams {
        workspace_dir: &state.workspace_dir,
        context_used: state.context_used,
        context_window: state.context_window,
        session_start: state.session_start,
        streaming: state.streaming,
    });
    frame.render_widget(Paragraph::new(status), status_chunk);
}

