// ABOUTME: Approval policy decision logic for tool invocations.
// ABOUTME: Evaluates SecurityLevel, AskMode, and allowlist status to produce an ApprovalOutcome.

use super::types::{AskMode, ApprovalOutcome, SecurityLevel};

/// Evaluate the approval policy for a tool invocation.
///
/// Given the security level, ask mode, and whether the allowlist is satisfied,
/// returns the appropriate approval outcome (Allow, Denied, or Ask).
pub fn evaluate_approval(
    security: SecurityLevel,
    ask: AskMode,
    allowlist_satisfied: bool,
) -> ApprovalOutcome {
    // Rule 1: Deny always blocks, regardless of anything else.
    if security == SecurityLevel::Deny {
        return ApprovalOutcome::Denied;
    }

    // Rule 2: Always-ask overrides all other logic.
    if ask == AskMode::Always {
        return ApprovalOutcome::Ask;
    }

    match security {
        SecurityLevel::Deny => unreachable!("handled above"),

        SecurityLevel::Allowlist => {
            if allowlist_satisfied {
                // Rule 3: Allowlist + satisfied → allow.
                ApprovalOutcome::Allow
            } else {
                match ask {
                    AskMode::OnMiss => ApprovalOutcome::Ask,   // Rule 4
                    AskMode::Off => ApprovalOutcome::Denied,   // Rule 5
                    AskMode::Always => unreachable!("handled above"),
                }
            }
        }

        SecurityLevel::Full => {
            if allowlist_satisfied || ask == AskMode::Off {
                // Rule 6: Full + (satisfied or Off) → allow.
                ApprovalOutcome::Allow
            } else {
                // Rule 7: Full + OnMiss + !satisfied → ask.
                ApprovalOutcome::Ask
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_always_blocks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Deny, AskMode::Off, true),
            ApprovalOutcome::Denied,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Deny, AskMode::Always, true),
            ApprovalOutcome::Denied,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Deny, AskMode::OnMiss, false),
            ApprovalOutcome::Denied,
        );
    }

    #[test]
    fn allowlist_satisfied_allows() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Off, true),
            ApprovalOutcome::Allow,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::OnMiss, true),
            ApprovalOutcome::Allow,
        );
    }

    #[test]
    fn allowlist_miss_with_on_miss_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::OnMiss, false),
            ApprovalOutcome::Ask,
        );
    }

    #[test]
    fn allowlist_miss_with_off_denies() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Off, false),
            ApprovalOutcome::Denied,
        );
    }

    #[test]
    fn allowlist_with_always_ask_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Always, true),
            ApprovalOutcome::Ask,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Always, false),
            ApprovalOutcome::Ask,
        );
    }

    #[test]
    fn full_with_off_allows() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Off, false),
            ApprovalOutcome::Allow,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Off, true),
            ApprovalOutcome::Allow,
        );
    }

    #[test]
    fn full_with_always_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Always, true),
            ApprovalOutcome::Ask,
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Always, false),
            ApprovalOutcome::Ask,
        );
    }

    #[test]
    fn full_with_on_miss_allows_when_satisfied() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::OnMiss, true),
            ApprovalOutcome::Allow,
        );
    }

    #[test]
    fn full_with_on_miss_asks_when_not_satisfied() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::OnMiss, false),
            ApprovalOutcome::Ask,
        );
    }
}
