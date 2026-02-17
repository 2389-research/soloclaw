// ABOUTME: E2E tests for TUI rendering using ratatui's TestBackend.
// ABOUTME: Verifies the TUI renders chat messages, status bar, and approval prompts.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use soloclaw::tui::state::{ChatMessageKind, TuiState};
use soloclaw::tui::ui;

/// Extract a single row of text from the terminal buffer as a String.
fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
    let buf = terminal.backend().buffer();
    let width = buf.area.width;
    (0..width)
        .map(|x| {
            buf.cell((x, y))
                .map(|c| c.symbol().chars().next().unwrap_or(' '))
                .unwrap_or(' ')
        })
        .collect()
}

/// Extract all text from the terminal buffer as a single string (rows joined by newlines).
fn all_text(terminal: &Terminal<TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let height = buf.area.height;
    (0..height)
        .map(|y| row_text(terminal, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rendering an empty TuiState should produce a header line containing
/// "soloclaw" and the model name, verifying the full rendering pipeline
/// from state through layout to buffer output.
#[test]
fn renders_empty_state() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 5);

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    let header = row_text(&terminal, 0);
    assert!(
        header.contains("soloclaw"),
        "header should contain 'soloclaw', got: {:?}",
        header,
    );
}

/// After pushing a User message, the rendered buffer should contain
/// the "❯" prefix and the message text, confirming the full render
/// pipeline processes chat messages end-to-end.
#[test]
fn renders_user_message() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 0);
    state.push_message(ChatMessageKind::User, "Hello agent!".to_string());

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    let text = all_text(&terminal);
    assert!(
        text.contains("❯"),
        "rendered output should contain '❯', got:\n{}",
        text,
    );
    assert!(
        text.contains("Hello agent!"),
        "rendered output should contain 'Hello agent!', got:\n{}",
        text,
    );
}

/// The status bar (last row, y=23 on an 80x24 terminal) should display
/// the model name, tool count, and formatted token count, verifying
/// that TuiState metrics flow through to the status bar widget.
#[test]
fn renders_status_bar() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("claude-sonnet".to_string(), 12);
    state.total_tokens = 1500;

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    // Status bar is at the bottom row (y=23 in 0-indexed for a 24-row terminal).
    let status = row_text(&terminal, 23);
    assert!(
        status.contains("1.5k"),
        "status bar should contain '1.5k' for 1500 tokens, got: {:?}",
        status,
    );
    assert!(
        status.contains("12 tools"),
        "status bar should contain '12 tools', got: {:?}",
        status,
    );
    assert!(
        status.contains("claude-sonnet"),
        "status bar should contain 'claude-sonnet', got: {:?}",
        status,
    );
}

/// Wrapped chat lines should contribute to scroll bounds so long responses
/// don't appear clipped by the input area.
#[test]
fn scroll_clamp_accounts_for_wrapped_chat_height() {
    let backend = TestBackend::new(24, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 0);
    state.push_message(
        ChatMessageKind::Assistant,
        "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega".to_string(),
    );
    state.scroll_offset = 100;

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    assert!(
        state.scroll_offset > 0,
        "scroll offset should clamp above zero when wrapped content exceeds chat viewport",
    );
}

/// With scroll_offset at 0 (auto-scroll mode), incremental assistant text updates
/// should keep the viewport pinned to the newest content at the bottom.
#[test]
fn auto_scroll_stays_pinned_to_bottom_during_streaming_updates() {
    let backend = TestBackend::new(24, 10);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 0);
    state.push_message(
        ChatMessageKind::Assistant,
        "line1\nline2\nline3\nline4".to_string(),
    );

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    state.append_to_last_assistant("\nline5\nline6\nline7\nline8");

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    let text = all_text(&terminal);
    assert!(
        text.contains("line8"),
        "viewport should include newest bottom content, got:\n{}",
        text,
    );
    assert!(
        !text.contains("line1"),
        "viewport should have scrolled past oldest content, got:\n{}",
        text,
    );
}

/// Cursor should be clamped to the input viewport when the input text exceeds available width.
#[test]
fn cursor_is_clamped_inside_input_viewport_for_long_input() {
    let backend = TestBackend::new(12, 8);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 0);
    state.input = "abcdefghijklmnopqrstuvwxyz".to_string();
    state.cursor_pos = state.input.chars().count();

    terminal
        .draw(|frame| ui::render(frame, &mut state))
        .unwrap();

    let cursor = terminal.get_cursor_position().unwrap();
    assert!(
        cursor.x < 12,
        "cursor x should stay within terminal width, got {:?}",
        cursor,
    );
}
