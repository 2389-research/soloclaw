// ABOUTME: E2E tests for TUI rendering using ratatui's TestBackend.
// ABOUTME: Verifies the TUI renders chat messages, status bar, and approval prompts.

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use simpleclaw::tui::state::{ChatMessageKind, TuiState};
use simpleclaw::tui::ui;

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
/// "simpleclaw" and the model name, verifying the full rendering pipeline
/// from state through layout to buffer output.
#[test]
fn renders_empty_state() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let state = TuiState::new("test-model".to_string(), 5);

    terminal
        .draw(|frame| ui::render(frame, &state))
        .unwrap();

    let header = row_text(&terminal, 0);
    assert!(
        header.contains("simpleclaw"),
        "header should contain 'simpleclaw', got: {:?}",
        header,
    );
    assert!(
        header.contains("test-model"),
        "header should contain 'test-model', got: {:?}",
        header,
    );
    assert!(
        header.contains("5 tools"),
        "header should contain '5 tools', got: {:?}",
        header,
    );
}

/// After pushing a User message, the rendered buffer should contain
/// the "You:" prefix and the message text, confirming the full render
/// pipeline processes chat messages end-to-end.
#[test]
fn renders_user_message() {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    let mut state = TuiState::new("test-model".to_string(), 0);
    state.push_message(ChatMessageKind::User, "Hello agent!".to_string());

    terminal
        .draw(|frame| ui::render(frame, &state))
        .unwrap();

    let text = all_text(&terminal);
    assert!(
        text.contains("You:"),
        "rendered output should contain 'You:', got:\n{}",
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
        .draw(|frame| ui::render(frame, &state))
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
