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
    /// User wants to quit.
    Quit,
}

/// Process a key event against the current TUI state and return the resulting action.
pub fn handle_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return InputResult::Quit;
    }

    // If there's a pending approval, route to approval handler
    if state.has_pending_approval() {
        return handle_approval_key(state, key);
    }

    // If streaming, ignore all input
    if state.streaming {
        return InputResult::None;
    }

    // Normal input mode
    match key.code {
        KeyCode::Enter => {
            if let Some(text) = state.submit_input() {
                InputResult::Send(text)
            } else {
                InputResult::None
            }
        }
        KeyCode::Char(c) => {
            state.input.insert(state.cursor_pos, c);
            state.cursor_pos += 1;
            InputResult::None
        }
        KeyCode::Backspace => {
            if state.cursor_pos > 0 {
                state.cursor_pos -= 1;
                state.input.remove(state.cursor_pos);
            }
            InputResult::None
        }
        KeyCode::Delete => {
            if state.cursor_pos < state.input.len() {
                state.input.remove(state.cursor_pos);
            }
            InputResult::None
        }
        KeyCode::Left => {
            state.cursor_pos = state.cursor_pos.saturating_sub(1);
            InputResult::None
        }
        KeyCode::Right => {
            if state.cursor_pos < state.input.len() {
                state.cursor_pos += 1;
            }
            InputResult::None
        }
        KeyCode::Home => {
            state.cursor_pos = 0;
            InputResult::None
        }
        KeyCode::End => {
            state.cursor_pos = state.input.len();
            InputResult::None
        }
        KeyCode::Up => {
            state.scroll_offset = state.scroll_offset.saturating_add(1);
            InputResult::None
        }
        KeyCode::Down => {
            state.scroll_offset = state.scroll_offset.saturating_sub(1);
            InputResult::None
        }
        KeyCode::Esc => InputResult::Quit,
        _ => InputResult::None,
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
            let decision = state
                .pending_approval
                .as_ref()
                .map(|a| match a.selected {
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
}
