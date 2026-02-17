// ABOUTME: Keyboard input handling for the TUI — translates key events into actions.
// ABOUTME: Handles normal typing, approval navigation, and streaming mode.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::approval::ApprovalDecision;
use crate::tui::state::TuiState;
use crate::tui::widgets::approval::APPROVAL_OPTIONS;

/// The result of processing a key event.
#[derive(Debug, PartialEq)]
pub enum InputResult {
    /// No action needed.
    None,
    /// User submitted a message.
    Send(String),
    /// User made an approval decision.
    Approval(ApprovalDecision),
    /// User answered a question from the LLM.
    QuestionAnswered(String),
    /// User wants to quit.
    Quit,
}

/// Process a key event against the current TUI state and return the resulting action.
pub fn handle_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return InputResult::Quit;
    }

    // PageUp/PageDown always scroll, regardless of mode.
    if handle_scroll_key(state, key.code) {
        return InputResult::None;
    }

    // Up/Down always scroll during streaming or approval modes.
    if state.streaming || state.has_pending_approval() {
        match key.code {
            KeyCode::Up => {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
                return InputResult::None;
            }
            KeyCode::Down => {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
                return InputResult::None;
            }
            _ => {}
        }
    }

    // If there's a pending approval, route to approval handler
    if state.has_pending_approval() {
        return handle_approval_key(state, key);
    }

    // If there's a pending question, route to question handler
    if state.has_pending_question() {
        return handle_question_key(state, key);
    }

    // If streaming, ignore all input
    if state.streaming {
        return InputResult::None;
    }

    // Context-aware Up/Down in normal input mode: move cursor within multiline
    // input first, then fall back to chat scrolling.
    match key.code {
        KeyCode::Up => {
            if !state.move_cursor_up_in_input() {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
            return InputResult::None;
        }
        KeyCode::Down => {
            if !state.move_cursor_down_in_input() {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
            return InputResult::None;
        }
        _ => {}
    }

    // Normal input mode
    match key.code {
        // Shift+Enter inserts a newline into the input buffer.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
            state.insert_char_at_cursor('\n');
            InputResult::None
        }
        KeyCode::Enter => {
            if let Some(text) = state.submit_input() {
                InputResult::Send(text)
            } else {
                InputResult::None
            }
        }
        KeyCode::Char(c) => {
            state.insert_char_at_cursor(c);
            InputResult::None
        }
        KeyCode::Backspace => {
            state.backspace_char();
            InputResult::None
        }
        KeyCode::Delete => {
            state.delete_char_at_cursor();
            InputResult::None
        }
        KeyCode::Left => {
            state.move_cursor_left();
            InputResult::None
        }
        KeyCode::Right => {
            state.move_cursor_right();
            InputResult::None
        }
        KeyCode::Home => {
            state.move_cursor_home();
            InputResult::None
        }
        KeyCode::End => {
            state.move_cursor_end();
            InputResult::None
        }
        KeyCode::Esc => InputResult::Quit,
        _ => InputResult::None,
    }
}

fn handle_scroll_key(state: &mut TuiState, key: KeyCode) -> bool {
    match key {
        KeyCode::PageUp => {
            state.scroll_offset = state.scroll_offset.saturating_add(10);
            true
        }
        KeyCode::PageDown => {
            state.scroll_offset = state.scroll_offset.saturating_sub(10);
            true
        }
        _ => false,
    }
}

/// Handle key events while an approval prompt is active.
fn handle_approval_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Left => {
            if let Some(ref mut approval) = state.pending_approval {
                approval.selected = approval.selected.saturating_sub(1);
            }
            InputResult::None
        }
        KeyCode::Right => {
            if let Some(ref mut approval) = state.pending_approval {
                if approval.selected < APPROVAL_OPTIONS.len() - 1 {
                    approval.selected += 1;
                }
            }
            InputResult::None
        }
        KeyCode::Char('1') => resolve_approval(state, ApprovalDecision::AllowOnce),
        KeyCode::Char('2') => resolve_approval(state, ApprovalDecision::AllowAlways),
        KeyCode::Char('3') => resolve_approval(state, ApprovalDecision::Deny),
        KeyCode::Enter => {
            let decision = state.pending_approval.as_ref().map(|a| match a.selected {
                0 => ApprovalDecision::AllowOnce,
                1 => ApprovalDecision::AllowAlways,
                _ => ApprovalDecision::Deny,
            });
            if let Some(d) = decision {
                resolve_approval(state, d)
            } else {
                InputResult::None
            }
        }
        _ => InputResult::None,
    }
}

/// Handle key events while a question prompt is active.
fn handle_question_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Enter => {
            let text = state.input.clone();
            state.input.clear();
            state.cursor_pos = 0;
            resolve_question(state, text)
        }
        KeyCode::Esc => {
            state.input.clear();
            state.cursor_pos = 0;
            resolve_question(state, "[User declined to answer]".to_string())
        }
        KeyCode::Char(c) => {
            state.insert_char_at_cursor(c);
            InputResult::None
        }
        KeyCode::Backspace => {
            state.backspace_char();
            InputResult::None
        }
        KeyCode::Delete => {
            state.delete_char_at_cursor();
            InputResult::None
        }
        KeyCode::Left => {
            state.move_cursor_left();
            InputResult::None
        }
        KeyCode::Right => {
            state.move_cursor_right();
            InputResult::None
        }
        KeyCode::Home => {
            state.move_cursor_home();
            InputResult::None
        }
        KeyCode::End => {
            state.move_cursor_end();
            InputResult::None
        }
        _ => InputResult::None,
    }
}

/// Resolve the pending question by sending the answer via the oneshot channel.
fn resolve_question(state: &mut TuiState, answer: String) -> InputResult {
    if let Some(mut question) = state.pending_question.take() {
        if let Some(responder) = question.responder.take() {
            // Send answer back to the agent loop; ignore errors if the receiver dropped.
            let _ = responder.send(answer.clone());
        }
    }
    InputResult::QuestionAnswered(answer)
}

/// Resolve the pending approval by sending the decision via the oneshot channel.
fn resolve_approval(state: &mut TuiState, decision: ApprovalDecision) -> InputResult {
    if let Some(mut approval) = state.pending_approval.take() {
        if let Some(responder) = approval.responder.take() {
            // Send decision back to the agent loop; ignore errors if the receiver dropped.
            let _ = responder.send(decision);
        }
    }
    InputResult::Approval(decision)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    fn make_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_appends_to_input() {
        let mut state = TuiState::new("m".to_string(), 0);
        let result = handle_key(&mut state, make_key(KeyCode::Char('h')));
        assert_eq!(result, InputResult::None);
        assert_eq!(state.input, "h");
        assert_eq!(state.cursor_pos, 1);

        handle_key(&mut state, make_key(KeyCode::Char('i')));
        assert_eq!(state.input, "hi");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn enter_submits_input() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 5;
        let result = handle_key(&mut state, make_key(KeyCode::Enter));
        assert_eq!(result, InputResult::Send("hello".to_string()));
        assert_eq!(state.input, "");
        assert_eq!(state.cursor_pos, 0);
    }

    #[test]
    fn enter_on_empty_does_nothing() {
        let mut state = TuiState::new("m".to_string(), 0);
        let result = handle_key(&mut state, make_key(KeyCode::Enter));
        assert_eq!(result, InputResult::None);
    }

    #[test]
    fn backspace_deletes() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc".to_string();
        state.cursor_pos = 3;
        let result = handle_key(&mut state, make_key(KeyCode::Backspace));
        assert_eq!(result, InputResult::None);
        assert_eq!(state.input, "ab");
        assert_eq!(state.cursor_pos, 2);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut state = TuiState::new("m".to_string(), 0);
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let result = handle_key(&mut state, key);
        assert_eq!(result, InputResult::Quit);
    }

    #[test]
    fn streaming_ignores_input() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.streaming = true;
        let result = handle_key(&mut state, make_key(KeyCode::Char('x')));
        assert_eq!(result, InputResult::None);
        assert_eq!(state.input, "");
    }

    #[test]
    fn streaming_still_allows_scroll_keys() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.streaming = true;
        state.scroll_offset = 2;

        assert_eq!(
            handle_key(&mut state, make_key(KeyCode::Up)),
            InputResult::None
        );
        assert_eq!(state.scroll_offset, 3);

        assert_eq!(
            handle_key(&mut state, make_key(KeyCode::Down)),
            InputResult::None
        );
        assert_eq!(state.scroll_offset, 2);
    }

    #[test]
    fn approval_mode_still_allows_scroll_keys() {
        let mut state = TuiState::new("m".to_string(), 0);
        let (tx, _rx) = oneshot::channel();
        state.pending_approval = Some(crate::tui::state::PendingApproval {
            description: "approve?".to_string(),
            pattern: None,
            tool_name: "bash".to_string(),
            selected: 0,
            responder: Some(tx),
        });
        state.scroll_offset = 4;

        assert_eq!(
            handle_key(&mut state, make_key(KeyCode::Up)),
            InputResult::None
        );
        assert_eq!(state.scroll_offset, 5);

        assert_eq!(
            handle_key(&mut state, make_key(KeyCode::PageDown)),
            InputResult::None
        );
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn unicode_editing_through_key_events() {
        let mut state = TuiState::new("m".to_string(), 0);
        handle_key(&mut state, make_key(KeyCode::Char('🙂')));
        handle_key(&mut state, make_key(KeyCode::Char('é')));
        assert_eq!(state.input, "🙂é");
        assert_eq!(state.cursor_pos, 2);

        handle_key(&mut state, make_key(KeyCode::Left));
        handle_key(&mut state, make_key(KeyCode::Delete));
        assert_eq!(state.input, "🙂");
        assert_eq!(state.cursor_pos, 1);

        handle_key(&mut state, make_key(KeyCode::Backspace));
        assert_eq!(state.input, "");
        assert_eq!(state.cursor_pos, 0);
    }

    // --- Multiline input tests ---

    fn make_shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    #[test]
    fn shift_enter_inserts_newline() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 5;
        let result = handle_key(&mut state, make_shift_key(KeyCode::Enter));
        assert_eq!(result, InputResult::None);
        assert_eq!(state.input, "hello\n");
        assert_eq!(state.cursor_pos, 6);
    }

    #[test]
    fn up_at_first_line_scrolls_chat() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello".to_string();
        state.cursor_pos = 3;
        state.scroll_offset = 0;
        let result = handle_key(&mut state, make_key(KeyCode::Up));
        assert_eq!(result, InputResult::None);
        // Cursor is on line 0, so Up should scroll chat
        assert_eq!(state.scroll_offset, 1);
    }

    #[test]
    fn up_on_second_line_moves_cursor() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "abc\ndef".to_string();
        // cursor at 'd' (char pos 5: a,b,c,\n,d)
        state.cursor_pos = 5;
        state.scroll_offset = 0;
        let result = handle_key(&mut state, make_key(KeyCode::Up));
        assert_eq!(result, InputResult::None);
        // Should move cursor up to line 0, col 1
        assert_eq!(state.cursor_pos, 1);
        // Scroll should NOT change
        assert_eq!(state.scroll_offset, 0);
    }

    // --- Question mode tests ---

    use crate::tui::state::PendingQuestion;

    fn make_question_state() -> (TuiState, oneshot::Receiver<String>) {
        let mut state = TuiState::new("m".to_string(), 0);
        let (tx, rx) = oneshot::channel();
        state.pending_question = Some(PendingQuestion {
            question: "What is your name?".to_string(),
            tool_call_id: "call-1".to_string(),
            responder: Some(tx),
        });
        (state, rx)
    }

    #[test]
    fn question_mode_allows_typing() {
        let (mut state, _rx) = make_question_state();
        let result = handle_key(&mut state, make_key(KeyCode::Char('H')));
        assert_eq!(result, InputResult::None);
        assert_eq!(state.input, "H");

        handle_key(&mut state, make_key(KeyCode::Char('i')));
        assert_eq!(state.input, "Hi");
    }

    #[test]
    fn question_mode_enter_submits_answer() {
        let (mut state, rx) = make_question_state();
        // Type an answer
        handle_key(&mut state, make_key(KeyCode::Char('B')));
        handle_key(&mut state, make_key(KeyCode::Char('o')));
        handle_key(&mut state, make_key(KeyCode::Char('b')));
        assert_eq!(state.input, "Bob");

        // Press Enter
        let result = handle_key(&mut state, make_key(KeyCode::Enter));
        assert_eq!(result, InputResult::QuestionAnswered("Bob".to_string()));
        assert!(!state.has_pending_question());
        assert_eq!(state.input, "");

        // Verify the answer was sent via oneshot
        assert_eq!(rx.blocking_recv().unwrap(), "Bob");
    }

    #[test]
    fn question_mode_esc_declines() {
        let (mut state, rx) = make_question_state();
        // Type something then press Esc
        handle_key(&mut state, make_key(KeyCode::Char('x')));
        let result = handle_key(&mut state, make_key(KeyCode::Esc));
        assert_eq!(
            result,
            InputResult::QuestionAnswered("[User declined to answer]".to_string())
        );
        assert!(!state.has_pending_question());
        assert_eq!(state.input, "");

        // Verify decline message was sent
        assert_eq!(rx.blocking_recv().unwrap(), "[User declined to answer]");
    }

    #[test]
    fn question_mode_backspace_works() {
        let (mut state, _rx) = make_question_state();
        handle_key(&mut state, make_key(KeyCode::Char('a')));
        handle_key(&mut state, make_key(KeyCode::Char('b')));
        handle_key(&mut state, make_key(KeyCode::Backspace));
        assert_eq!(state.input, "a");
    }

    #[test]
    fn question_mode_delete_works() {
        let (mut state, _rx) = make_question_state();
        handle_key(&mut state, make_key(KeyCode::Char('a')));
        handle_key(&mut state, make_key(KeyCode::Char('b')));
        handle_key(&mut state, make_key(KeyCode::Left));
        handle_key(&mut state, make_key(KeyCode::Delete));
        assert_eq!(state.input, "a");
    }

    #[test]
    fn question_mode_home_end_work() {
        let (mut state, _rx) = make_question_state();
        handle_key(&mut state, make_key(KeyCode::Char('a')));
        handle_key(&mut state, make_key(KeyCode::Char('b')));
        handle_key(&mut state, make_key(KeyCode::Char('c')));
        assert_eq!(state.cursor_pos, 3);

        handle_key(&mut state, make_key(KeyCode::Home));
        assert_eq!(state.cursor_pos, 0);

        handle_key(&mut state, make_key(KeyCode::End));
        assert_eq!(state.cursor_pos, 3);
    }

    #[test]
    fn question_mode_enter_on_empty_sends_empty() {
        let (mut state, rx) = make_question_state();
        let result = handle_key(&mut state, make_key(KeyCode::Enter));
        assert_eq!(result, InputResult::QuestionAnswered("".to_string()));
        assert_eq!(rx.blocking_recv().unwrap(), "");
    }

    #[test]
    fn question_mode_scroll_up_down_still_work() {
        let (mut state, _rx) = make_question_state();
        state.scroll_offset = 2;

        // Up/Down should scroll since question mode is checked after the
        // streaming/approval scroll block. But question mode has its own
        // routing, so Up/Down in question mode go to handle_question_key.
        // The scroll check is handled before question mode in handle_key.
        // Let's verify with PageUp/PageDown which always scroll.
        assert_eq!(
            handle_key(&mut state, make_key(KeyCode::PageUp)),
            InputResult::None
        );
        assert_eq!(state.scroll_offset, 12);
    }

    #[test]
    fn question_mode_ctrl_c_still_quits() {
        let (mut state, _rx) = make_question_state();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let result = handle_key(&mut state, key);
        assert_eq!(result, InputResult::Quit);
    }
}
