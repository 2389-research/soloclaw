# soloclaw Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a full-screen ratatui TUI agent powered by mux-rs with an openclaw-style layered tool approval system.

**Architecture:** Standalone Rust binary with three core modules: `approval/` (layered security engine), `tui/` (ratatui full-screen UI), and `agent/` (LLM streaming + tool dispatch). Communication between TUI (main thread) and agent loop (tokio task) via mpsc channels. Approval requests use oneshot channels for synchronous resolution.

**Tech Stack:** Rust (edition 2024), mux-rs (path dep), ratatui + crossterm, tokio, clap, serde/toml/serde_json

**Reference files:**
- Design doc: `docs/plans/2026-02-16-soloclaw-design.md`
- mux-rs prelude: `../mux-rs/src/prelude.rs` — all public types
- mux-rs agent-test-tui: `../mux-rs/agent-test-tui/src/main.rs` — reference agent loop
- openclaw approval: see design doc for pattern description

---

### Task 1: Project Scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`

**Step 1: Create Cargo.toml**

```toml
[package]
name = "soloclaw"
version = "0.1.0"
edition = "2024"

[dependencies]
mux = { path = "../mux-rs" }
ratatui = "0.29"
crossterm = "0.28"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
dirs = "5"
anyhow = "1"
dotenvy = "0.15"
glob = "0.3"
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
futures = "0.3"
```

**Step 2: Create minimal main.rs**

```rust
// ABOUTME: Entry point for soloclaw — a TUI agent with layered tool approval.
// ABOUTME: Parses CLI args, loads config, and launches the app.

fn main() {
    println!("soloclaw");
}
```

**Step 3: Verify it compiles**

Run: `cd /Users/harper/Public/src/2389/soloclaw && cargo build`
Expected: Compiles successfully (may take a while for first build)

**Step 4: Commit**

```bash
git add Cargo.toml src/main.rs
git commit -m "feat: scaffold soloclaw project with dependencies"
```

---

### Task 2: Approval Types

**Files:**
- Create: `src/approval/mod.rs`
- Create: `src/approval/types.rs`
- Modify: `src/main.rs` (add module declaration)

**Step 1: Write the failing test**

Create `src/approval/types.rs` with types AND tests:

```rust
// ABOUTME: Core types for the layered approval system.
// ABOUTME: SecurityLevel, AskMode, AskFallback, and ApprovalDecision enums.

use serde::{Deserialize, Serialize};

/// How restrictive the security policy is for a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityLevel {
    /// Reject all invocations unconditionally.
    Deny,
    /// Allow only invocations matching the allowlist.
    Allowlist,
    /// Allow all invocations (subject to ask mode).
    Full,
}

/// When to prompt the user for approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AskMode {
    /// Never ask — auto-decide based on security level.
    Off,
    /// Ask only when the allowlist doesn't cover the invocation.
    OnMiss,
    /// Always ask, regardless of allowlist.
    Always,
}

/// What to do when an approval request times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskFallback {
    /// Deny on timeout.
    Deny,
    /// Fall back to allowlist check on timeout.
    Allowlist,
    /// Allow on timeout.
    Full,
}

/// The user's decision on an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Allow this one invocation.
    AllowOnce,
    /// Allow and add to persistent allowlist.
    AllowAlways,
    /// Deny this invocation.
    Deny,
}

/// The outcome of evaluating an approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Tool call is allowed without asking.
    Allow,
    /// Tool call is denied without asking.
    Denied,
    /// User must be prompted for approval.
    Ask,
}

/// Per-tool security configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSecurity {
    pub security: SecurityLevel,
    pub ask: AskMode,
    #[serde(default = "default_ask_fallback")]
    pub ask_fallback: AskFallback,
}

fn default_ask_fallback() -> AskFallback {
    AskFallback::Deny
}

impl Default for ToolSecurity {
    fn default() -> Self {
        Self {
            security: SecurityLevel::Allowlist,
            ask: AskMode::OnMiss,
            ask_fallback: AskFallback::Deny,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_level_serde_roundtrip() {
        let json = serde_json::to_string(&SecurityLevel::Allowlist).unwrap();
        assert_eq!(json, "\"allowlist\"");
        let parsed: SecurityLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SecurityLevel::Allowlist);
    }

    #[test]
    fn ask_mode_serde_roundtrip() {
        let json = serde_json::to_string(&AskMode::OnMiss).unwrap();
        assert_eq!(json, "\"on-miss\"");
        let parsed: AskMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AskMode::OnMiss);
    }

    #[test]
    fn tool_security_defaults() {
        let ts = ToolSecurity::default();
        assert_eq!(ts.security, SecurityLevel::Allowlist);
        assert_eq!(ts.ask, AskMode::OnMiss);
        assert_eq!(ts.ask_fallback, AskFallback::Deny);
    }

    #[test]
    fn tool_security_from_json() {
        let json = r#"{"security":"full","ask":"always"}"#;
        let ts: ToolSecurity = serde_json::from_str(json).unwrap();
        assert_eq!(ts.security, SecurityLevel::Full);
        assert_eq!(ts.ask, AskMode::Always);
        assert_eq!(ts.ask_fallback, AskFallback::Deny); // default
    }
}
```

**Step 2: Create mod.rs and wire into main**

Create `src/approval/mod.rs`:

```rust
// ABOUTME: Layered tool approval system inspired by openclaw.
// ABOUTME: Security levels, ask modes, persistent allowlists, and command analysis.

pub mod types;

pub use types::*;
```

Update `src/main.rs` to add the module:

```rust
// ABOUTME: Entry point for soloclaw — a TUI agent with layered tool approval.
// ABOUTME: Parses CLI args, loads config, and launches the app.

mod approval;

fn main() {
    println!("soloclaw");
}
```

**Step 3: Run tests to verify they pass**

Run: `cargo test -p soloclaw approval::types`
Expected: 4 tests pass

**Step 4: Commit**

```bash
git add src/approval/
git commit -m "feat: add approval types — SecurityLevel, AskMode, ApprovalDecision"
```

---

### Task 3: Approval Policy (Decision Logic)

**Files:**
- Create: `src/approval/policy.rs`
- Modify: `src/approval/mod.rs` (add pub mod)

This is the core decision function, equivalent to openclaw's `requiresExecApproval`.

**Step 1: Write the failing test first in policy.rs**

```rust
// ABOUTME: Approval policy decision logic.
// ABOUTME: Evaluates security level + ask mode + allowlist status to produce an outcome.

use super::types::{AskMode, ApprovalOutcome, SecurityLevel};

/// Evaluate whether a tool call requires approval.
///
/// Arguments:
/// - `security`: The security level for this tool
/// - `ask`: The ask mode for this tool
/// - `allowlist_satisfied`: Whether the tool/command matches the allowlist
pub fn evaluate_approval(
    security: SecurityLevel,
    ask: AskMode,
    allowlist_satisfied: bool,
) -> ApprovalOutcome {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_always_blocks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Deny, AskMode::Off, false),
            ApprovalOutcome::Denied
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Deny, AskMode::Always, true),
            ApprovalOutcome::Denied
        );
    }

    #[test]
    fn allowlist_satisfied_allows() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::OnMiss, true),
            ApprovalOutcome::Allow
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Off, true),
            ApprovalOutcome::Allow
        );
    }

    #[test]
    fn allowlist_miss_with_on_miss_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::OnMiss, false),
            ApprovalOutcome::Ask
        );
    }

    #[test]
    fn allowlist_miss_with_off_denies() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Off, false),
            ApprovalOutcome::Denied
        );
    }

    #[test]
    fn allowlist_with_always_ask_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Always, true),
            ApprovalOutcome::Ask
        );
        assert_eq!(
            evaluate_approval(SecurityLevel::Allowlist, AskMode::Always, false),
            ApprovalOutcome::Ask
        );
    }

    #[test]
    fn full_with_off_allows() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Off, false),
            ApprovalOutcome::Allow
        );
    }

    #[test]
    fn full_with_always_asks() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::Always, false),
            ApprovalOutcome::Ask
        );
    }

    #[test]
    fn full_with_on_miss_allows_when_satisfied() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::OnMiss, true),
            ApprovalOutcome::Allow
        );
    }

    #[test]
    fn full_with_on_miss_asks_when_not_satisfied() {
        assert_eq!(
            evaluate_approval(SecurityLevel::Full, AskMode::OnMiss, false),
            ApprovalOutcome::Ask
        );
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p soloclaw approval::policy`
Expected: FAIL — `todo!()` panics

**Step 3: Implement the decision logic**

Replace `todo!()` with:

```rust
pub fn evaluate_approval(
    security: SecurityLevel,
    ask: AskMode,
    allowlist_satisfied: bool,
) -> ApprovalOutcome {
    // Layer 1: Deny blocks everything
    if security == SecurityLevel::Deny {
        return ApprovalOutcome::Denied;
    }

    // Layer 2: Always-ask overrides allowlist
    if ask == AskMode::Always {
        return ApprovalOutcome::Ask;
    }

    // Layer 3: Allowlist mode
    if security == SecurityLevel::Allowlist {
        if allowlist_satisfied {
            return ApprovalOutcome::Allow;
        }
        return match ask {
            AskMode::OnMiss => ApprovalOutcome::Ask,
            AskMode::Off => ApprovalOutcome::Denied,
            AskMode::Always => unreachable!(), // handled above
        };
    }

    // Layer 4: Full mode
    // security == SecurityLevel::Full
    if allowlist_satisfied || ask == AskMode::Off {
        return ApprovalOutcome::Allow;
    }

    // Full + OnMiss + not satisfied
    ApprovalOutcome::Ask
}
```

**Step 4: Run tests to verify they pass**

Run: `cargo test -p soloclaw approval::policy`
Expected: 9 tests pass

**Step 5: Add to mod.rs and commit**

Add `pub mod policy;` and `pub use policy::*;` to `src/approval/mod.rs`.

```bash
git add src/approval/policy.rs src/approval/mod.rs
git commit -m "feat: add approval policy decision logic with full test coverage"
```

---

### Task 4: Persistent Allowlist

**Files:**
- Create: `src/approval/allowlist.rs`
- Modify: `src/approval/mod.rs`

**Step 1: Write the failing tests**

```rust
// ABOUTME: Persistent allowlist for tool approval patterns.
// ABOUTME: Loads from and saves to ~/.soloclaw/approvals.json.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::types::{AskFallback, AskMode, SecurityLevel, ToolSecurity};

/// A single allowlist entry — a pattern that has been approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistEntry {
    pub pattern: String,
    pub added_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_command: Option<String>,
}

/// Per-tool configuration with its allowlist entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolApprovalConfig {
    #[serde(flatten)]
    pub security: ToolSecurity,
    #[serde(default)]
    pub allowlist: Vec<AllowlistEntry>,
}

/// Root approvals file structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalsFile {
    pub version: u32,
    pub defaults: ToolSecurity,
    #[serde(default)]
    pub tools: HashMap<String, ToolApprovalConfig>,
}

impl Default for ApprovalsFile {
    fn default() -> Self {
        Self {
            version: 1,
            defaults: ToolSecurity::default(),
            tools: HashMap::new(),
        }
    }
}

impl ApprovalsFile {
    /// Load from a file path, or return defaults if not found.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let file: Self = serde_json::from_str(&content)?;
        Ok(file)
    }

    /// Save to a file path, creating parent dirs if needed.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get the security config for a tool, falling back to wildcard then defaults.
    pub fn tool_security(&self, tool_name: &str) -> ToolSecurity {
        if let Some(config) = self.tools.get(tool_name) {
            return config.security.clone();
        }
        if let Some(config) = self.tools.get("*") {
            return config.security.clone();
        }
        self.defaults.clone()
    }

    /// Check if a pattern is in the allowlist for a tool.
    pub fn is_allowed(&self, tool_name: &str, pattern: &str) -> bool {
        if let Some(config) = self.tools.get(tool_name) {
            return config.allowlist.iter().any(|e| {
                glob::Pattern::new(&e.pattern)
                    .map(|p| p.matches(pattern))
                    .unwrap_or(e.pattern == pattern)
            });
        }
        false
    }

    /// Add a pattern to a tool's allowlist.
    pub fn add_to_allowlist(&mut self, tool_name: &str, pattern: String) {
        let config = self.tools.entry(tool_name.to_string()).or_insert_with(|| {
            ToolApprovalConfig {
                security: self.defaults.clone(),
                allowlist: Vec::new(),
            }
        });

        // Don't add duplicates
        if config.allowlist.iter().any(|e| e.pattern == pattern) {
            return;
        }

        config.allowlist.push(AllowlistEntry {
            pattern,
            added_at: Utc::now(),
            last_used_at: None,
            last_used_command: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_approvals_file() {
        let f = ApprovalsFile::default();
        assert_eq!(f.version, 1);
        assert_eq!(f.defaults.security, SecurityLevel::Allowlist);
        assert!(f.tools.is_empty());
    }

    #[test]
    fn tool_security_falls_back_to_defaults() {
        let f = ApprovalsFile::default();
        let sec = f.tool_security("anything");
        assert_eq!(sec.security, SecurityLevel::Allowlist);
        assert_eq!(sec.ask, AskMode::OnMiss);
    }

    #[test]
    fn tool_security_uses_specific_config() {
        let mut f = ApprovalsFile::default();
        f.tools.insert(
            "bash".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Full,
                    ask: AskMode::Always,
                    ask_fallback: AskFallback::Deny,
                },
                allowlist: Vec::new(),
            },
        );
        let sec = f.tool_security("bash");
        assert_eq!(sec.security, SecurityLevel::Full);
        assert_eq!(sec.ask, AskMode::Always);
    }

    #[test]
    fn tool_security_wildcard_fallback() {
        let mut f = ApprovalsFile::default();
        f.tools.insert(
            "*".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Deny,
                    ask: AskMode::Off,
                    ask_fallback: AskFallback::Deny,
                },
                allowlist: Vec::new(),
            },
        );
        let sec = f.tool_security("unknown_tool");
        assert_eq!(sec.security, SecurityLevel::Deny);
    }

    #[test]
    fn allowlist_exact_match() {
        let mut f = ApprovalsFile::default();
        f.add_to_allowlist("bash", "/usr/bin/ls".to_string());
        assert!(f.is_allowed("bash", "/usr/bin/ls"));
        assert!(!f.is_allowed("bash", "/usr/bin/rm"));
    }

    #[test]
    fn allowlist_glob_match() {
        let mut f = ApprovalsFile::default();
        f.add_to_allowlist("bash", "/usr/bin/python*".to_string());
        assert!(f.is_allowed("bash", "/usr/bin/python3"));
        assert!(f.is_allowed("bash", "/usr/bin/python3.12"));
        assert!(!f.is_allowed("bash", "/usr/bin/ruby"));
    }

    #[test]
    fn allowlist_no_duplicates() {
        let mut f = ApprovalsFile::default();
        f.add_to_allowlist("bash", "/usr/bin/ls".to_string());
        f.add_to_allowlist("bash", "/usr/bin/ls".to_string());
        assert_eq!(f.tools["bash"].allowlist.len(), 1);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");

        let mut f = ApprovalsFile::default();
        f.add_to_allowlist("bash", "/usr/bin/ls".to_string());
        f.save(&path).unwrap();

        let loaded = ApprovalsFile::load(&path).unwrap();
        assert!(loaded.is_allowed("bash", "/usr/bin/ls"));
        assert_eq!(loaded.tools["bash"].allowlist.len(), 1);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let path = PathBuf::from("/tmp/nonexistent_soloclaw_test.json");
        let f = ApprovalsFile::load(&path).unwrap();
        assert_eq!(f.version, 1);
        assert!(f.tools.is_empty());
    }
}
```

Add `tempfile = "3"` to `[dev-dependencies]` in Cargo.toml.

**Step 2: Run tests to verify they pass**

Run: `cargo test -p soloclaw approval::allowlist`
Expected: 8 tests pass

**Step 3: Wire into mod.rs and commit**

Add `pub mod allowlist;` and `pub use allowlist::*;` to `src/approval/mod.rs`.

```bash
git add src/approval/allowlist.rs src/approval/mod.rs Cargo.toml
git commit -m "feat: add persistent allowlist with glob matching and JSON persistence"
```

---

### Task 5: Command Analysis

**Files:**
- Create: `src/approval/analysis.rs`
- Modify: `src/approval/mod.rs`

Shell command parsing and safe-bin detection for the bash tool.

**Step 1: Write the implementation with tests**

```rust
// ABOUTME: Static analysis of shell commands for approval decisions.
// ABOUTME: Parses pipelines, resolves executables, detects safe stdin-only binaries.

use std::path::PathBuf;

/// A parsed segment of a shell pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandSegment {
    /// The executable (first token).
    pub executable: String,
    /// The full argument list (including executable).
    pub args: Vec<String>,
    /// Whether this segment only receives stdin (piped into).
    pub stdin_only: bool,
}

/// Result of analyzing a shell command.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// The parsed pipeline segments.
    pub segments: Vec<CommandSegment>,
    /// Resolved absolute path of the primary executable (first segment).
    pub resolved_path: Option<PathBuf>,
    /// Whether static analysis considers this command safe.
    pub safe: bool,
}

/// Binaries that only process stdin and cannot escape.
const SAFE_BINS: &[&str] = &[
    "awk", "base64", "cat", "column", "cut", "diff", "echo", "env",
    "expand", "expr", "false", "fmt", "fold", "grep", "head", "jq",
    "less", "more", "nl", "od", "paste", "printf", "rev", "sed",
    "seq", "shuf", "sort", "strings", "tac", "tail", "tee", "tr",
    "true", "tsort", "uniq", "wc", "xargs", "yes",
];

/// Check if a binary name is in the safe-bins list.
pub fn is_safe_bin(name: &str) -> bool {
    let basename = name.rsplit('/').next().unwrap_or(name);
    SAFE_BINS.contains(&basename)
}

/// Try to resolve an executable name to an absolute path via PATH.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    // Already absolute
    if name.starts_with('/') {
        let path = PathBuf::from(name);
        if path.exists() {
            return Some(path);
        }
        return None;
    }

    // Search PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Parse a shell command string into pipeline segments.
/// Handles `&&`, `||`, `;`, and `|` operators.
pub fn parse_pipeline(command: &str) -> Vec<CommandSegment> {
    let mut segments = Vec::new();

    // Split on &&, ||, ; first (these are independent commands)
    // Then split on | (these are piped)
    for chain in command.split(&["&&", "||", ";"][..]) {
        let chain = chain.trim();
        if chain.is_empty() {
            continue;
        }

        let pipes: Vec<&str> = chain.split('|').collect();
        for (i, segment) in pipes.iter().enumerate() {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }

            let args: Vec<String> = shell_words(segment);
            if args.is_empty() {
                continue;
            }

            segments.push(CommandSegment {
                executable: args[0].clone(),
                args: args.clone(),
                stdin_only: i > 0,
            });
        }
    }

    segments
}

/// Minimal shell word splitting (handles basic quoting).
fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape_next = false;

    for ch in input.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }

        if ch == '\\' && !in_single_quote {
            escape_next = true;
            continue;
        }

        if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            continue;
        }

        if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            continue;
        }

        if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }

        current.push(ch);
    }

    if !current.is_empty() {
        words.push(current);
    }

    words
}

/// Analyze a shell command for approval purposes.
pub fn analyze_command(command: &str) -> AnalysisResult {
    let segments = parse_pipeline(command);

    if segments.is_empty() {
        return AnalysisResult {
            segments,
            resolved_path: None,
            safe: false,
        };
    }

    let primary = &segments[0];
    let resolved_path = resolve_executable(&primary.executable);

    // Safe if ALL segments are safe bins (except the first which can be anything
    // if the subsequent segments are all safe stdin-only processors)
    let all_piped_safe = segments.iter().skip(1).all(|s| {
        s.stdin_only && is_safe_bin(&s.executable)
    });

    // The primary command determines the resolved path for allowlist matching
    let safe = segments.len() == 1 && is_safe_bin(&primary.executable)
        || (segments.len() > 1 && is_safe_bin(&primary.executable) && all_piped_safe);

    AnalysisResult {
        segments,
        resolved_path,
        safe,
    }
}

/// Get the pattern to use for allowlist matching.
/// Prefers resolved absolute path, falls back to executable name.
pub fn allowlist_pattern(analysis: &AnalysisResult) -> Option<String> {
    if analysis.segments.is_empty() {
        return None;
    }

    if let Some(ref path) = analysis.resolved_path {
        Some(path.to_string_lossy().to_string())
    } else {
        Some(analysis.segments[0].executable.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_bin_recognizes_common_tools() {
        assert!(is_safe_bin("grep"));
        assert!(is_safe_bin("jq"));
        assert!(is_safe_bin("sort"));
        assert!(is_safe_bin("wc"));
        assert!(!is_safe_bin("rm"));
        assert!(!is_safe_bin("curl"));
        assert!(!is_safe_bin("python"));
    }

    #[test]
    fn is_safe_bin_handles_absolute_paths() {
        assert!(is_safe_bin("/usr/bin/grep"));
        assert!(!is_safe_bin("/usr/bin/rm"));
    }

    #[test]
    fn parse_simple_command() {
        let segs = parse_pipeline("ls -la");
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].executable, "ls");
        assert_eq!(segs[0].args, vec!["ls", "-la"]);
        assert!(!segs[0].stdin_only);
    }

    #[test]
    fn parse_pipeline_segments() {
        let segs = parse_pipeline("cat file.txt | grep error | wc -l");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].executable, "cat");
        assert!(!segs[0].stdin_only);
        assert_eq!(segs[1].executable, "grep");
        assert!(segs[1].stdin_only);
        assert_eq!(segs[2].executable, "wc");
        assert!(segs[2].stdin_only);
    }

    #[test]
    fn parse_chained_commands() {
        let segs = parse_pipeline("mkdir foo && cd foo");
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].executable, "mkdir");
        assert_eq!(segs[1].executable, "cd");
    }

    #[test]
    fn parse_quoted_args() {
        let segs = parse_pipeline(r#"echo "hello world" 'foo bar'"#);
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].args, vec!["echo", "hello world", "foo bar"]);
    }

    #[test]
    fn analyze_safe_pipeline() {
        let result = analyze_command("cat file.txt | grep error | wc -l");
        assert!(result.safe);
        assert_eq!(result.segments.len(), 3);
    }

    #[test]
    fn analyze_unsafe_command() {
        let result = analyze_command("rm -rf /");
        assert!(!result.safe);
    }

    #[test]
    fn analyze_mixed_pipeline_unsafe() {
        // rm piped to grep — first command is unsafe
        let result = analyze_command("find . -name '*.tmp' | xargs rm");
        assert!(!result.safe); // xargs is safe but rm through xargs is not our concern
        // Actually xargs IS in safe list, but find is not
        // The pipeline has find (unsafe) | xargs (safe, stdin_only)
        // safe = false because find is not a safe bin
    }

    #[test]
    fn allowlist_pattern_uses_resolved_path() {
        let result = analyze_command("ls");
        // ls should resolve to something on most systems
        if result.resolved_path.is_some() {
            let pattern = allowlist_pattern(&result).unwrap();
            assert!(pattern.starts_with('/'));
        }
    }

    #[test]
    fn allowlist_pattern_falls_back_to_name() {
        let result = analyze_command("nonexistent_binary_xyz");
        let pattern = allowlist_pattern(&result).unwrap();
        assert_eq!(pattern, "nonexistent_binary_xyz");
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p soloclaw approval::analysis`
Expected: All tests pass

**Step 3: Wire into mod.rs and commit**

Add `pub mod analysis;` and `pub use analysis::*;` to `src/approval/mod.rs`.

```bash
git add src/approval/analysis.rs src/approval/mod.rs
git commit -m "feat: add command analysis — shell parsing, safe-bin detection, PATH resolution"
```

---

### Task 6: Approval Engine

**Files:**
- Create: `src/approval/engine.rs`
- Modify: `src/approval/mod.rs`

The engine ties together policy, allowlist, and analysis. It owns the `ApprovalsFile` and provides the high-level `check_tool_call` method.

**Step 1: Write the implementation with tests**

```rust
// ABOUTME: ApprovalEngine orchestrates the full approval decision flow.
// ABOUTME: Combines policy evaluation, allowlist lookup, and command analysis.

use std::path::PathBuf;
use std::sync::Mutex;

use super::allowlist::ApprovalsFile;
use super::analysis::{allowlist_pattern, analyze_command};
use super::policy::evaluate_approval;
use super::types::{ApprovalDecision, ApprovalOutcome, ToolSecurity};

/// The approval engine — holds persistent state and evaluates tool calls.
pub struct ApprovalEngine {
    approvals: Mutex<ApprovalsFile>,
    approvals_path: PathBuf,
}

/// Information about a tool call being evaluated.
pub struct ToolCallInfo {
    pub tool_name: String,
    pub params: serde_json::Value,
}

/// Result of the engine's evaluation.
pub enum EngineOutcome {
    /// Allowed without asking.
    Allowed,
    /// Denied without asking.
    Denied { reason: String },
    /// Needs user approval. Contains a display-friendly description.
    NeedsApproval { description: String, pattern: Option<String> },
}

impl ApprovalEngine {
    /// Create a new engine, loading approvals from the given path.
    pub fn new(approvals_path: PathBuf) -> anyhow::Result<Self> {
        let approvals = ApprovalsFile::load(&approvals_path)?;
        Ok(Self {
            approvals: Mutex::new(approvals),
            approvals_path,
        })
    }

    /// Create an engine with an in-memory approvals file (for testing).
    pub fn with_approvals(approvals: ApprovalsFile, path: PathBuf) -> Self {
        Self {
            approvals: Mutex::new(approvals),
            approvals_path: path,
        }
    }

    /// Evaluate whether a tool call should proceed.
    pub fn check(&self, info: &ToolCallInfo) -> EngineOutcome {
        let approvals = self.approvals.lock().unwrap();
        let tool_sec = approvals.tool_security(&info.tool_name);

        // For bash tool, do command analysis
        let (allowlist_satisfied, pattern) = if info.tool_name == "bash" {
            self.check_bash(&approvals, &info.params)
        } else {
            let satisfied = approvals.is_allowed(&info.tool_name, &info.tool_name);
            (satisfied, Some(info.tool_name.clone()))
        };

        let outcome = evaluate_approval(tool_sec.security, tool_sec.ask, allowlist_satisfied);

        match outcome {
            ApprovalOutcome::Allow => EngineOutcome::Allowed,
            ApprovalOutcome::Denied => EngineOutcome::Denied {
                reason: format!(
                    "Tool '{}' denied by security policy ({:?})",
                    info.tool_name, tool_sec.security
                ),
            },
            ApprovalOutcome::Ask => {
                let description = self.describe_tool_call(info);
                EngineOutcome::NeedsApproval { description, pattern }
            }
        }
    }

    /// Handle an approval decision — persist if AllowAlways.
    pub fn resolve(&self, tool_name: &str, pattern: Option<&str>, decision: ApprovalDecision) {
        if decision == ApprovalDecision::AllowAlways {
            if let Some(pat) = pattern {
                let mut approvals = self.approvals.lock().unwrap();
                approvals.add_to_allowlist(tool_name, pat.to_string());
                let _ = approvals.save(&self.approvals_path);
            }
        }
    }

    fn check_bash(
        &self,
        approvals: &ApprovalsFile,
        params: &serde_json::Value,
    ) -> (bool, Option<String>) {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let analysis = analyze_command(command);

        // If all segments are safe bins, auto-approve
        if analysis.safe {
            return (true, allowlist_pattern(&analysis));
        }

        // Check allowlist with resolved path
        let pattern = allowlist_pattern(&analysis);
        if let Some(ref pat) = pattern {
            let satisfied = approvals.is_allowed("bash", pat);
            return (satisfied, Some(pat.clone()));
        }

        (false, None)
    }

    fn describe_tool_call(&self, info: &ToolCallInfo) -> String {
        if info.tool_name == "bash" {
            let command = info
                .params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            format!("bash(\"{}\")", command)
        } else {
            let params_summary = serde_json::to_string(&info.params)
                .unwrap_or_else(|_| "...".to_string());
            let truncated = if params_summary.len() > 60 {
                format!("{}...", &params_summary[..57])
            } else {
                params_summary
            };
            format!("{}({})", info.tool_name, truncated)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::types::*;
    use crate::approval::allowlist::ToolApprovalConfig;

    fn test_engine() -> ApprovalEngine {
        let mut approvals = ApprovalsFile::default();
        // bash: allowlist mode, on-miss ask
        approvals.tools.insert(
            "bash".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Allowlist,
                    ask: AskMode::OnMiss,
                    ask_fallback: AskFallback::Deny,
                },
                allowlist: Vec::new(),
            },
        );
        // read_file: full access, no ask
        approvals.tools.insert(
            "read_file".to_string(),
            ToolApprovalConfig {
                security: ToolSecurity {
                    security: SecurityLevel::Full,
                    ask: AskMode::Off,
                    ask_fallback: AskFallback::Deny,
                },
                allowlist: Vec::new(),
            },
        );
        ApprovalEngine::with_approvals(approvals, PathBuf::from("/tmp/test_approvals.json"))
    }

    #[test]
    fn bash_safe_command_auto_approves() {
        let engine = test_engine();
        let info = ToolCallInfo {
            tool_name: "bash".to_string(),
            params: serde_json::json!({"command": "cat file.txt | grep error"}),
        };
        assert!(matches!(engine.check(&info), EngineOutcome::Allowed));
    }

    #[test]
    fn bash_unsafe_command_asks() {
        let engine = test_engine();
        let info = ToolCallInfo {
            tool_name: "bash".to_string(),
            params: serde_json::json!({"command": "rm -rf /tmp/data"}),
        };
        assert!(matches!(engine.check(&info), EngineOutcome::NeedsApproval { .. }));
    }

    #[test]
    fn read_file_auto_approves() {
        let engine = test_engine();
        let info = ToolCallInfo {
            tool_name: "read_file".to_string(),
            params: serde_json::json!({"path": "/some/file"}),
        };
        assert!(matches!(engine.check(&info), EngineOutcome::Allowed));
    }

    #[test]
    fn unknown_tool_uses_defaults() {
        let engine = test_engine();
        let info = ToolCallInfo {
            tool_name: "some_new_tool".to_string(),
            params: serde_json::json!({}),
        };
        // Default is allowlist + on-miss, tool not in allowlist → ask
        assert!(matches!(engine.check(&info), EngineOutcome::NeedsApproval { .. }));
    }

    #[test]
    fn resolve_allow_always_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(ApprovalsFile::default(), path.clone());

        engine.resolve("bash", Some("/usr/bin/ls"), ApprovalDecision::AllowAlways);

        // Verify it was persisted
        let loaded = ApprovalsFile::load(&path).unwrap();
        assert!(loaded.is_allowed("bash", "/usr/bin/ls"));
    }

    #[test]
    fn resolve_allow_once_does_not_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("approvals.json");
        let engine = ApprovalEngine::with_approvals(ApprovalsFile::default(), path.clone());

        engine.resolve("bash", Some("/usr/bin/rm"), ApprovalDecision::AllowOnce);

        let loaded = ApprovalsFile::load(&path).unwrap();
        assert!(!loaded.is_allowed("bash", "/usr/bin/rm"));
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p soloclaw approval::engine`
Expected: 6 tests pass

**Step 3: Wire into mod.rs and commit**

Add `pub mod engine;` and `pub use engine::*;` to `src/approval/mod.rs`.

```bash
git add src/approval/engine.rs src/approval/mod.rs
git commit -m "feat: add ApprovalEngine — orchestrates policy, allowlist, and analysis"
```

---

### Task 7: Configuration Loading

**Files:**
- Create: `src/config.rs`
- Modify: `src/main.rs`

**Step 1: Write the implementation with tests**

```rust
// ABOUTME: Configuration loading for soloclaw.
// ABOUTME: Reads ~/.soloclaw/config.toml, .mcp.json, and CLI overrides.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use mux::prelude::*;

/// Top-level configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub llm: LlmConfig,
    pub approval: ApprovalConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            llm: LlmConfig::default(),
            approval: ApprovalConfig::default(),
        }
    }
}

/// LLM provider configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub ollama: OllamaConfig,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            ollama: OllamaConfig::default(),
        }
    }
}

/// Ollama-specific configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub base_url: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:11434".to_string(),
        }
    }
}

/// Approval defaults in config (overridden by approvals.json per-tool settings).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApprovalConfig {
    pub security: String,
    pub ask: String,
    pub ask_fallback: String,
    pub timeout_seconds: u64,
}

impl Default for ApprovalConfig {
    fn default() -> Self {
        Self {
            security: "allowlist".to_string(),
            ask: "on-miss".to_string(),
            ask_fallback: "deny".to_string(),
            timeout_seconds: 120,
        }
    }
}

/// MCP server configuration from .mcp.json (same format as mux-rs).
#[derive(Debug, Deserialize)]
struct McpConfigFile {
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, McpServerEntry>,
}

#[derive(Debug, Deserialize)]
struct McpServerEntry {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

impl Config {
    /// Load config from ~/.soloclaw/config.toml, falling back to defaults.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Path to the config file.
    pub fn config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".soloclaw")
            .join("config.toml")
    }

    /// Path to the approvals file.
    pub fn approvals_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".soloclaw")
            .join("approvals.json")
    }
}

/// Load MCP server configs from .mcp.json.
pub fn load_mcp_configs() -> anyhow::Result<Vec<McpServerConfig>> {
    let path = find_mcp_config();
    let Some(path) = path else {
        return Ok(vec![]);
    };

    let content = std::fs::read_to_string(&path)?;
    let config: McpConfigFile = serde_json::from_str(&content)?;

    let servers = config
        .mcp_servers
        .into_iter()
        .map(|(name, entry)| McpServerConfig {
            name,
            transport: McpTransport::Stdio {
                command: entry.command,
                args: entry.args,
                env: entry.env,
            },
        })
        .collect();

    Ok(servers)
}

fn find_mcp_config() -> Option<PathBuf> {
    let local = PathBuf::from(".mcp.json");
    if local.exists() {
        return Some(local);
    }

    if let Some(home) = dirs::home_dir() {
        let global = home.join(".mcp.json");
        if global.exists() {
            return Some(global);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let config = Config::default();
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.max_tokens, 4096);
        assert_eq!(config.approval.timeout_seconds, 120);
    }

    #[test]
    fn parse_config_toml() {
        let toml_str = r#"
[llm]
provider = "ollama"
model = "llama3"
max_tokens = 2048

[llm.ollama]
base_url = "http://localhost:11434"

[approval]
security = "full"
ask = "always"
timeout_seconds = 60
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "ollama");
        assert_eq!(config.llm.model, "llama3");
        assert_eq!(config.llm.max_tokens, 2048);
        assert_eq!(config.approval.security, "full");
        assert_eq!(config.approval.ask, "always");
        assert_eq!(config.approval.timeout_seconds, 60);
    }

    #[test]
    fn parse_partial_config_uses_defaults() {
        let toml_str = r#"
[llm]
provider = "openai"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.llm.model, "claude-sonnet-4-20250514"); // default
        assert_eq!(config.approval.timeout_seconds, 120); // default
    }
}
```

**Step 2: Run tests**

Run: `cargo test -p soloclaw config`
Expected: 3 tests pass

**Step 3: Wire into main.rs and commit**

Add `mod config;` to `src/main.rs`.

```bash
git add src/config.rs src/main.rs
git commit -m "feat: add config loading — TOML config and MCP JSON parsing"
```

---

### Task 8: LLM Provider Factory

**Files:**
- Create: `src/agent/mod.rs`
- Create: `src/agent/provider.rs`
- Modify: `src/main.rs`

**Step 1: Write the implementation**

```rust
// ABOUTME: LLM provider factory — creates the right client based on config.
// ABOUTME: Supports anthropic, openai, gemini, openrouter, and ollama.

use std::sync::Arc;

use mux::llm::{
    AnthropicClient, GeminiClient, LlmClient, OllamaClient, OpenAIClient, OpenRouterClient,
};

use crate::config::LlmConfig;

/// Create an LLM client based on the provider name in config.
pub fn create_client(config: &LlmConfig) -> anyhow::Result<Arc<dyn LlmClient>> {
    match config.provider.as_str() {
        "anthropic" => {
            let client = AnthropicClient::from_env()?;
            Ok(Arc::new(client))
        }
        "openai" => {
            let client = OpenAIClient::from_env()?;
            Ok(Arc::new(client))
        }
        "gemini" => {
            let client = GeminiClient::from_env()?;
            Ok(Arc::new(client))
        }
        "openrouter" => {
            let client = OpenRouterClient::from_env()?;
            Ok(Arc::new(client))
        }
        "ollama" => {
            let client = OllamaClient::new(&config.model)
                .with_base_url(&config.ollama.base_url);
            Ok(Arc::new(client))
        }
        other => anyhow::bail!("Unknown LLM provider: '{}'. Expected one of: anthropic, openai, gemini, openrouter, ollama", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_provider_errors() {
        let config = LlmConfig {
            provider: "fakeprovider".to_string(),
            ..Default::default()
        };
        let result = create_client(&config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("fakeprovider"));
    }
}
```

Note: need to check if OllamaClient has a `with_base_url` method. If not, adjust.

Create `src/agent/mod.rs`:

```rust
// ABOUTME: Agent module — LLM provider factory and streaming agent loop.
// ABOUTME: Manages conversation history and tool call dispatch.

pub mod provider;

pub use provider::*;
```

**Step 2: Run tests**

Run: `cargo test -p soloclaw agent::provider`
Expected: 1 test passes

**Step 3: Wire into main.rs and commit**

Add `mod agent;` to `src/main.rs`.

```bash
git add src/agent/ src/main.rs
git commit -m "feat: add LLM provider factory — supports all mux-rs backends"
```

---

### Task 9: TUI State & Event Types

**Files:**
- Create: `src/tui/mod.rs`
- Create: `src/tui/state.rs`
- Modify: `src/main.rs`

**Step 1: Write the implementation with tests**

```rust
// ABOUTME: TUI state management — messages, scroll position, input buffer.
// ABOUTME: Defines the chat message model and event types for agent ↔ TUI communication.

use tokio::sync::oneshot;

use crate::approval::ApprovalDecision;

/// A message in the chat history.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub kind: ChatMessageKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatMessageKind {
    User,
    Assistant,
    /// A tool call with its approval status.
    ToolCall {
        tool_name: String,
        status: ToolCallStatus,
    },
    /// Output from a tool execution.
    ToolResult {
        is_error: bool,
    },
    /// System message (errors, status updates).
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStatus {
    Allowed,
    Denied,
    Pending,
    TimedOut,
}

/// Events sent from the agent loop to the TUI.
#[derive(Debug)]
pub enum AgentEvent {
    /// A text delta from streaming.
    TextDelta(String),
    /// Streaming for current message is complete.
    TextDone,
    /// A tool call is being evaluated.
    ToolCallStarted {
        tool_name: String,
        params_summary: String,
    },
    /// Tool call was auto-approved and is executing.
    ToolCallApproved { tool_name: String },
    /// Tool call needs user approval.
    ToolCallNeedsApproval {
        description: String,
        pattern: Option<String>,
        tool_name: String,
        responder: oneshot::Sender<ApprovalDecision>,
    },
    /// Tool call was denied by policy.
    ToolCallDenied { tool_name: String, reason: String },
    /// Tool execution completed.
    ToolResult {
        tool_name: String,
        content: String,
        is_error: bool,
    },
    /// An error occurred in the agent loop.
    Error(String),
    /// Agent loop finished processing (waiting for next input).
    Done,
}

/// Events sent from the TUI to the agent loop.
#[derive(Debug)]
pub enum UserEvent {
    /// User submitted a message.
    Message(String),
    /// User wants to quit.
    Quit,
}

/// The full TUI state.
pub struct TuiState {
    /// Chat messages.
    pub messages: Vec<ChatMessage>,
    /// Current input buffer.
    pub input: String,
    /// Cursor position in input.
    pub cursor_pos: usize,
    /// Scroll offset (0 = bottom).
    pub scroll_offset: u16,
    /// Whether we're currently streaming a response.
    pub streaming: bool,
    /// Current approval prompt, if any.
    pub pending_approval: Option<PendingApproval>,
    /// Model name for status bar.
    pub model: String,
    /// Tool count for status bar.
    pub tool_count: usize,
    /// Token usage for status bar.
    pub total_tokens: u64,
}

/// An active approval prompt.
pub struct PendingApproval {
    pub description: String,
    pub pattern: Option<String>,
    pub tool_name: String,
    pub selected: usize, // 0=AllowOnce, 1=AllowAlways, 2=Deny
    pub responder: Option<oneshot::Sender<ApprovalDecision>>,
}

impl TuiState {
    pub fn new(model: String, tool_count: usize) -> Self {
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            scroll_offset: 0,
            streaming: false,
            pending_approval: None,
            model,
            tool_count,
            total_tokens: 0,
        }
    }

    /// Add a chat message.
    pub fn push_message(&mut self, kind: ChatMessageKind, content: String) {
        self.messages.push(ChatMessage { kind, content });
        self.scroll_offset = 0; // auto-scroll to bottom
    }

    /// Append text to the last assistant message (for streaming).
    pub fn append_to_last_assistant(&mut self, text: &str) {
        if let Some(last) = self.messages.last_mut() {
            if last.kind == ChatMessageKind::Assistant {
                last.content.push_str(text);
                return;
            }
        }
        // No existing assistant message — create one
        self.push_message(ChatMessageKind::Assistant, text.to_string());
    }

    /// Submit the current input, returning it and clearing the buffer.
    pub fn submit_input(&mut self) -> Option<String> {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.input.clear();
        self.cursor_pos = 0;
        Some(text)
    }

    /// Whether we're waiting for an approval decision.
    pub fn has_pending_approval(&self) -> bool {
        self.pending_approval.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_state_is_empty() {
        let state = TuiState::new("test-model".to_string(), 5);
        assert!(state.messages.is_empty());
        assert!(state.input.is_empty());
        assert!(!state.streaming);
        assert!(!state.has_pending_approval());
        assert_eq!(state.model, "test-model");
        assert_eq!(state.tool_count, 5);
    }

    #[test]
    fn push_message_auto_scrolls() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.scroll_offset = 10;
        state.push_message(ChatMessageKind::User, "hello".to_string());
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.messages.len(), 1);
    }

    #[test]
    fn append_to_streaming_message() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.push_message(ChatMessageKind::Assistant, "Hello".to_string());
        state.append_to_last_assistant(", world!");
        assert_eq!(state.messages.last().unwrap().content, "Hello, world!");
    }

    #[test]
    fn append_creates_new_if_no_assistant() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.push_message(ChatMessageKind::User, "hi".to_string());
        state.append_to_last_assistant("response");
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[1].kind, ChatMessageKind::Assistant);
    }

    #[test]
    fn submit_input_clears_buffer() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hello world".to_string();
        state.cursor_pos = 11;
        let submitted = state.submit_input();
        assert_eq!(submitted, Some("hello world".to_string()));
        assert!(state.input.is_empty());
        assert_eq!(state.cursor_pos, 0);
    }

    #[test]
    fn submit_empty_input_returns_none() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "   ".to_string();
        assert_eq!(state.submit_input(), None);
    }
}
```

Create `src/tui/mod.rs`:

```rust
// ABOUTME: TUI module — ratatui full-screen interface for soloclaw.
// ABOUTME: Chat display, input handling, status bar, and inline approval prompts.

pub mod state;

pub use state::*;
```

**Step 2: Run tests**

Run: `cargo test -p soloclaw tui::state`
Expected: 6 tests pass

**Step 3: Wire into main.rs and commit**

Add `mod tui;` to `src/main.rs`.

```bash
git add src/tui/ src/main.rs
git commit -m "feat: add TUI state — chat messages, events, input buffer, approval state"
```

---

### Task 10: TUI Widgets — Chat, Status Bar, Approval

**Files:**
- Create: `src/tui/widgets/mod.rs`
- Create: `src/tui/widgets/chat.rs`
- Create: `src/tui/widgets/status.rs`
- Create: `src/tui/widgets/approval.rs`
- Modify: `src/tui/mod.rs`

**Step 1: Create chat widget**

`src/tui/widgets/chat.rs`:

```rust
// ABOUTME: Chat widget — renders conversation messages in the main area.
// ABOUTME: Handles user messages, assistant text, tool calls, and tool results.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::text::{Line, Span};

use crate::tui::state::{ChatMessage, ChatMessageKind, ToolCallStatus};

/// Render the chat messages into styled lines.
pub fn render_chat_lines(messages: &[ChatMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for msg in messages {
        match &msg.kind {
            ChatMessageKind::User => {
                lines.push(Line::from(vec![
                    Span::styled("You: ", Style::default().fg(Color::Green).bold()),
                    Span::raw(msg.content.clone()),
                ]));
                lines.push(Line::from(""));
            }
            ChatMessageKind::Assistant => {
                lines.push(Line::from(vec![
                    Span::styled("Assistant: ", Style::default().fg(Color::Cyan).bold()),
                ]));
                for text_line in msg.content.lines() {
                    lines.push(Line::from(Span::raw(format!("  {}", text_line))));
                }
                lines.push(Line::from(""));
            }
            ChatMessageKind::ToolCall { tool_name, status } => {
                let status_span = match status {
                    ToolCallStatus::Allowed => {
                        Span::styled("[allowed]", Style::default().fg(Color::Green))
                    }
                    ToolCallStatus::Denied => {
                        Span::styled("[denied]", Style::default().fg(Color::Red))
                    }
                    ToolCallStatus::Pending => {
                        Span::styled("[pending]", Style::default().fg(Color::Yellow))
                    }
                    ToolCallStatus::TimedOut => {
                        Span::styled("[timed out]", Style::default().fg(Color::DarkGray))
                    }
                };
                lines.push(Line::from(vec![
                    Span::styled("  >> ", Style::default().fg(Color::Yellow)),
                    Span::styled(tool_name.clone(), Style::default().fg(Color::Yellow).bold()),
                    Span::raw(format!("({}) ", truncate(&msg.content, 50))),
                    status_span,
                ]));
            }
            ChatMessageKind::ToolResult { is_error } => {
                let style = if *is_error {
                    Style::default().fg(Color::Red)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                for text_line in msg.content.lines().take(10) {
                    lines.push(Line::from(Span::styled(
                        format!("  | {}", text_line),
                        style,
                    )));
                }
                let line_count = msg.content.lines().count();
                if line_count > 10 {
                    lines.push(Line::from(Span::styled(
                        format!("  | ... ({} more lines)", line_count - 10),
                        style,
                    )));
                }
                lines.push(Line::from(""));
            }
            ChatMessageKind::System => {
                lines.push(Line::from(Span::styled(
                    format!("  [system] {}", msg.content),
                    Style::default().fg(Color::DarkGray).italic(),
                )));
                lines.push(Line::from(""));
            }
        }
    }

    lines
}

/// Render the full chat widget.
pub fn chat_widget(messages: &[ChatMessage], scroll_offset: u16) -> Paragraph<'static> {
    let lines = render_chat_lines(messages);
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false })
        .scroll((scroll_offset, 0))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
```

**Step 2: Create status bar widget**

`src/tui/widgets/status.rs`:

```rust
// ABOUTME: Status bar widget — model name, token count, tool count, streaming indicator.
// ABOUTME: Rendered at the bottom of the TUI.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};

/// Render the status bar line.
pub fn status_line(
    model: &str,
    tool_count: usize,
    total_tokens: u64,
    streaming: bool,
) -> Line<'static> {
    let streaming_indicator = if streaming {
        Span::styled(" streaming ", Style::default().fg(Color::Yellow).bold())
    } else {
        Span::styled(" ready ", Style::default().fg(Color::Green))
    };

    Line::from(vec![
        Span::styled(
            format!(" {} ", model),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ),
        Span::styled(
            format!(" tokens: {} ", format_tokens(total_tokens)),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ),
        Span::styled(
            format!(" tools: {} ", tool_count),
            Style::default().fg(Color::White).bg(Color::DarkGray),
        ),
        streaming_indicator,
    ])
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(42), "42");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(1500), "1.5k");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }
}
```

**Step 3: Create approval widget**

`src/tui/widgets/approval.rs`:

```rust
// ABOUTME: Inline approval prompt widget.
// ABOUTME: Shows [Allow Once] [Always Allow] [Deny] with keyboard navigation.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};

/// The three approval options.
pub const APPROVAL_OPTIONS: &[&str] = &["Allow Once", "Always Allow", "Deny"];

/// Render the approval prompt as a line.
pub fn approval_line(description: &str, selected: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("  >> ", Style::default().fg(Color::Yellow)),
        Span::styled(description.to_string(), Style::default().fg(Color::Yellow).bold()),
        Span::styled("  APPROVE? ", Style::default().fg(Color::Yellow).bold()),
    ]));

    let mut option_spans = vec![Span::raw("     ")];
    for (i, option) in APPROVAL_OPTIONS.iter().enumerate() {
        let style = if i == selected {
            Style::default().fg(Color::Black).bg(Color::Yellow).bold()
        } else {
            Style::default().fg(Color::White)
        };
        option_spans.push(Span::styled(format!(" {} ", option), style));
        if i < APPROVAL_OPTIONS.len() - 1 {
            option_spans.push(Span::raw("  "));
        }
    }
    lines.push(Line::from(option_spans));

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_line_has_all_options() {
        let lines = approval_line("bash(\"rm -rf /\")", 0);
        assert_eq!(lines.len(), 2);
        // Just verify it doesn't panic and returns 2 lines
    }

    #[test]
    fn selected_index_is_valid() {
        // Each valid index should render without panicking
        for i in 0..3 {
            let _lines = approval_line("test", i);
        }
    }
}
```

**Step 4: Create widgets mod.rs**

`src/tui/widgets/mod.rs`:

```rust
// ABOUTME: TUI widget sub-modules for chat, status bar, and approval prompt.
// ABOUTME: Each widget is a pure rendering function over TuiState.

pub mod approval;
pub mod chat;
pub mod status;
```

Update `src/tui/mod.rs`:

```rust
// ABOUTME: TUI module — ratatui full-screen interface for soloclaw.
// ABOUTME: Chat display, input handling, status bar, and inline approval prompts.

pub mod state;
pub mod widgets;

pub use state::*;
```

**Step 5: Run tests**

Run: `cargo test -p soloclaw tui`
Expected: All widget tests pass

**Step 6: Commit**

```bash
git add src/tui/
git commit -m "feat: add TUI widgets — chat, status bar, and approval prompt rendering"
```

---

### Task 11: TUI Rendering & Input

**Files:**
- Create: `src/tui/ui.rs`
- Create: `src/tui/input.rs`
- Modify: `src/tui/mod.rs`

**Step 1: Create the main UI rendering function**

`src/tui/ui.rs`:

```rust
// ABOUTME: Main TUI layout and rendering — assembles widgets into the full screen.
// ABOUTME: Handles the header, chat area, input area, and status bar layout.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::text::{Line, Span};

use super::state::TuiState;
use super::widgets::{approval, chat, status};

/// Render the full TUI frame.
pub fn render(frame: &mut Frame, state: &TuiState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),   // header
            Constraint::Min(3),      // chat area
            Constraint::Length(3),   // input area
            Constraint::Length(1),   // status bar
        ])
        .split(frame.area());

    render_header(frame, chunks[0], state);
    render_chat(frame, chunks[1], state);
    render_input(frame, chunks[2], state);
    render_status(frame, chunks[3], state);
}

fn render_header(frame: &mut Frame, area: Rect, state: &TuiState) {
    let header = Line::from(vec![
        Span::styled(" soloclaw ", Style::default().fg(Color::White).bold()),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(&state.model, Style::default().fg(Color::Cyan)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} tools", state.tool_count),
            Style::default().fg(Color::Green),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(header).style(Style::default().bg(Color::DarkGray)),
        area,
    );
}

fn render_chat(frame: &mut Frame, area: Rect, state: &TuiState) {
    let mut lines = chat::render_chat_lines(&state.messages);

    // Append approval prompt if pending
    if let Some(ref approval_state) = state.pending_approval {
        let approval_lines = approval::approval_line(
            &approval_state.description,
            approval_state.selected,
        );
        lines.extend(approval_lines);
    }

    // Calculate scroll: we want to show the bottom of content
    let content_height = lines.len() as u16;
    let visible_height = area.height;
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll = if state.scroll_offset == 0 {
        max_scroll // auto-scroll to bottom
    } else {
        max_scroll.saturating_sub(state.scroll_offset)
    };

    let chat = Paragraph::new(lines)
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(chat, area);
}

fn render_input(frame: &mut Frame, area: Rect, state: &TuiState) {
    let input_text = if state.has_pending_approval() {
        "(approve/deny the tool call above)".to_string()
    } else if state.streaming {
        "(waiting for response...)".to_string()
    } else {
        state.input.clone()
    };

    let input = Paragraph::new(input_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if state.has_pending_approval() {
                    Color::Yellow
                } else {
                    Color::White
                }))
                .title(" Message "),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(input, area);

    // Place cursor
    if !state.has_pending_approval() && !state.streaming {
        frame.set_cursor_position((
            area.x + 1 + state.cursor_pos as u16,
            area.y + 1,
        ));
    }
}

fn render_status(frame: &mut Frame, area: Rect, state: &TuiState) {
    let line = status::status_line(
        &state.model,
        state.tool_count,
        state.total_tokens,
        state.streaming,
    );
    frame.render_widget(Paragraph::new(line), area);
}
```

**Step 2: Create input handling**

`src/tui/input.rs`:

```rust
// ABOUTME: Keyboard input handling for the TUI.
// ABOUTME: Maps key events to actions on TuiState.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::approval::ApprovalDecision;
use super::state::{TuiState, UserEvent};

/// Result of handling a key event.
pub enum InputResult {
    /// No action needed.
    None,
    /// User submitted a message.
    Send(String),
    /// User resolved an approval prompt.
    Approval(ApprovalDecision),
    /// User wants to quit.
    Quit,
}

/// Handle a key event, returning what action to take.
pub fn handle_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return InputResult::Quit;
    }

    // If there's a pending approval, handle approval keys
    if state.has_pending_approval() {
        return handle_approval_key(state, key);
    }

    // If streaming, ignore most input
    if state.streaming {
        return InputResult::None;
    }

    // Normal input mode
    match key.code {
        KeyCode::Enter => {
            if let Some(text) = state.submit_input() {
                return InputResult::Send(text);
            }
            InputResult::None
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
            state.cursor_pos = (state.cursor_pos + 1).min(state.input.len());
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

fn handle_approval_key(state: &mut TuiState, key: KeyEvent) -> InputResult {
    let approval = state.pending_approval.as_mut().unwrap();

    match key.code {
        KeyCode::Left => {
            approval.selected = approval.selected.saturating_sub(1);
            InputResult::None
        }
        KeyCode::Right => {
            approval.selected = (approval.selected + 1).min(2);
            InputResult::None
        }
        KeyCode::Char('1') => {
            let decision = ApprovalDecision::AllowOnce;
            resolve_approval(state, decision)
        }
        KeyCode::Char('2') => {
            let decision = ApprovalDecision::AllowAlways;
            resolve_approval(state, decision)
        }
        KeyCode::Char('3') => {
            let decision = ApprovalDecision::Deny;
            resolve_approval(state, decision)
        }
        KeyCode::Enter => {
            let decision = match approval.selected {
                0 => ApprovalDecision::AllowOnce,
                1 => ApprovalDecision::AllowAlways,
                _ => ApprovalDecision::Deny,
            };
            resolve_approval(state, decision)
        }
        _ => InputResult::None,
    }
}

fn resolve_approval(state: &mut TuiState, decision: ApprovalDecision) -> InputResult {
    if let Some(mut approval) = state.pending_approval.take() {
        if let Some(responder) = approval.responder.take() {
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
        handle_key(&mut state, make_key(KeyCode::Char('h')));
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
        assert!(matches!(result, InputResult::Send(s) if s == "hello"));
        assert!(state.input.is_empty());
    }

    #[test]
    fn enter_on_empty_does_nothing() {
        let mut state = TuiState::new("m".to_string(), 0);
        let result = handle_key(&mut state, make_key(KeyCode::Enter));
        assert!(matches!(result, InputResult::None));
    }

    #[test]
    fn backspace_deletes() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.input = "hi".to_string();
        state.cursor_pos = 2;
        handle_key(&mut state, make_key(KeyCode::Backspace));
        assert_eq!(state.input, "h");
        assert_eq!(state.cursor_pos, 1);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut state = TuiState::new("m".to_string(), 0);
        let result = handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(matches!(result, InputResult::Quit));
    }

    #[test]
    fn streaming_ignores_input() {
        let mut state = TuiState::new("m".to_string(), 0);
        state.streaming = true;
        let result = handle_key(&mut state, make_key(KeyCode::Char('a')));
        assert!(matches!(result, InputResult::None));
        assert!(state.input.is_empty());
    }
}
```

**Step 3: Update tui/mod.rs**

```rust
// ABOUTME: TUI module — ratatui full-screen interface for soloclaw.
// ABOUTME: Chat display, input handling, status bar, and inline approval prompts.

pub mod input;
pub mod state;
pub mod ui;
pub mod widgets;

pub use state::*;
```

**Step 4: Run tests**

Run: `cargo test -p soloclaw tui`
Expected: All tests pass

**Step 5: Commit**

```bash
git add src/tui/
git commit -m "feat: add TUI rendering and input handling"
```

---

### Task 12: Agent Loop

**Files:**
- Create: `src/agent/loop.rs`
- Modify: `src/agent/mod.rs`

The agent loop runs in a tokio task. It receives user messages, streams LLM responses, dispatches tool calls through the approval engine, and sends events to the TUI.

**Step 1: Write the implementation**

```rust
// ABOUTME: Streaming agent loop — receives user messages, calls LLM, dispatches tools.
// ABOUTME: Communicates with the TUI via mpsc channels and approval via oneshot.

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};

use mux::prelude::*;

use crate::approval::{ApprovalDecision, ApprovalEngine, EngineOutcome, ToolCallInfo};
use crate::tui::state::{AgentEvent, PendingApproval, UserEvent};

/// Run the agent loop in a background task.
pub async fn run_agent_loop(
    client: Arc<dyn LlmClient>,
    registry: Registry,
    engine: Arc<ApprovalEngine>,
    model: String,
    max_tokens: u32,
    mut user_rx: mpsc::Receiver<UserEvent>,
    agent_tx: mpsc::Sender<AgentEvent>,
) {
    let mut history: Vec<Message> = Vec::new();

    while let Some(event) = user_rx.recv().await {
        match event {
            UserEvent::Quit => break,
            UserEvent::Message(text) => {
                history.push(Message::user(&text));

                if let Err(e) = run_turn(
                    &client,
                    &registry,
                    &engine,
                    &model,
                    max_tokens,
                    &mut history,
                    &agent_tx,
                )
                .await
                {
                    let _ = agent_tx.send(AgentEvent::Error(e.to_string())).await;
                }

                let _ = agent_tx.send(AgentEvent::Done).await;
            }
        }
    }
}

/// Run a single turn: call LLM, handle tool calls, repeat until done.
async fn run_turn(
    client: &Arc<dyn LlmClient>,
    registry: &Registry,
    engine: &Arc<ApprovalEngine>,
    model: &str,
    max_tokens: u32,
    history: &mut Vec<Message>,
    agent_tx: &mpsc::Sender<AgentEvent>,
) -> anyhow::Result<()> {
    loop {
        let request = Request::new(model)
            .messages(history.clone())
            .tools(registry.to_definitions().await)
            .max_tokens(max_tokens);

        // Stream the response
        let mut stream = client.create_message_stream(&request);
        let mut response_content: Vec<ContentBlock> = Vec::new();
        let mut current_tool_json = String::new();
        let mut has_tool_use = false;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::ContentBlockDelta { text, .. }) => {
                    let _ = agent_tx.send(AgentEvent::TextDelta(text)).await;
                }
                Ok(StreamEvent::ContentBlockStart { block, .. }) => {
                    if let ContentBlock::ToolUse { ref name, .. } = block {
                        has_tool_use = true;
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallStarted {
                                tool_name: name.clone(),
                                params_summary: String::new(),
                            })
                            .await;
                    }
                    response_content.push(block);
                    current_tool_json.clear();
                }
                Ok(StreamEvent::InputJsonDelta { partial_json, .. }) => {
                    current_tool_json.push_str(&partial_json);
                }
                Ok(StreamEvent::ContentBlockStop { index }) => {
                    // If this was a tool use block, update the input with accumulated JSON
                    if let Some(ContentBlock::ToolUse { ref mut input, .. }) =
                        response_content.get_mut(index)
                    {
                        if !current_tool_json.is_empty() {
                            if let Ok(parsed) = serde_json::from_str(&current_tool_json) {
                                *input = parsed;
                            }
                        }
                    }
                }
                Ok(StreamEvent::MessageDelta { usage, .. }) => {
                    // Token usage tracking could be sent to TUI here
                    let _ = usage;
                }
                Ok(StreamEvent::TextDone | StreamEvent::MessageStop) => {}
                Ok(_) => {}
                Err(e) => {
                    let _ = agent_tx.send(AgentEvent::Error(e.to_string())).await;
                    return Ok(());
                }
            }
        }

        let _ = agent_tx.send(AgentEvent::TextDone).await;

        // Add assistant message to history
        history.push(Message {
            role: Role::Assistant,
            content: response_content.clone(),
        });

        // If no tool calls, we're done
        if !has_tool_use {
            return Ok(());
        }

        // Process tool calls
        let mut tool_results = Vec::new();

        for block in &response_content {
            if let ContentBlock::ToolUse { id, name, input } = block {
                let tool_call_info = ToolCallInfo {
                    tool_name: name.clone(),
                    params: input.clone(),
                };

                let outcome = engine.check(&tool_call_info);

                match outcome {
                    EngineOutcome::Allowed => {
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallApproved {
                                tool_name: name.clone(),
                            })
                            .await;
                        let result = execute_tool(registry, name, input.clone()).await;
                        let _ = agent_tx
                            .send(AgentEvent::ToolResult {
                                tool_name: name.clone(),
                                content: result.content.clone(),
                                is_error: result.is_error,
                            })
                            .await;

                        if result.is_error {
                            tool_results.push(ContentBlock::tool_error(id, &result.content));
                        } else {
                            tool_results.push(ContentBlock::tool_result(id, &result.content));
                        }
                    }
                    EngineOutcome::Denied { reason } => {
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallDenied {
                                tool_name: name.clone(),
                                reason: reason.clone(),
                            })
                            .await;
                        tool_results.push(ContentBlock::tool_error(
                            id,
                            format!("Tool call denied: {}", reason),
                        ));
                    }
                    EngineOutcome::NeedsApproval {
                        description,
                        pattern,
                    } => {
                        // Send approval request to TUI and wait
                        let (tx, rx) = oneshot::channel();
                        let _ = agent_tx
                            .send(AgentEvent::ToolCallNeedsApproval {
                                description: description.clone(),
                                pattern: pattern.clone(),
                                tool_name: name.clone(),
                                responder: tx,
                            })
                            .await;

                        // Wait for user decision (with timeout)
                        let decision = tokio::time::timeout(
                            std::time::Duration::from_secs(120),
                            rx,
                        )
                        .await;

                        match decision {
                            Ok(Ok(decision)) => {
                                engine.resolve(name, pattern.as_deref(), decision);

                                if decision == ApprovalDecision::Deny {
                                    let _ = agent_tx
                                        .send(AgentEvent::ToolCallDenied {
                                            tool_name: name.clone(),
                                            reason: "Denied by user".to_string(),
                                        })
                                        .await;
                                    tool_results.push(ContentBlock::tool_error(
                                        id,
                                        "Tool call denied by user",
                                    ));
                                } else {
                                    let _ = agent_tx
                                        .send(AgentEvent::ToolCallApproved {
                                            tool_name: name.clone(),
                                        })
                                        .await;
                                    let result =
                                        execute_tool(registry, name, input.clone()).await;
                                    let _ = agent_tx
                                        .send(AgentEvent::ToolResult {
                                            tool_name: name.clone(),
                                            content: result.content.clone(),
                                            is_error: result.is_error,
                                        })
                                        .await;
                                    if result.is_error {
                                        tool_results
                                            .push(ContentBlock::tool_error(id, &result.content));
                                    } else {
                                        tool_results
                                            .push(ContentBlock::tool_result(id, &result.content));
                                    }
                                }
                            }
                            _ => {
                                // Timeout or channel error
                                let _ = agent_tx
                                    .send(AgentEvent::ToolCallDenied {
                                        tool_name: name.clone(),
                                        reason: "Approval timed out".to_string(),
                                    })
                                    .await;
                                tool_results.push(ContentBlock::tool_error(
                                    id,
                                    "Tool call denied: approval timed out",
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Add tool results to history and loop back
        history.push(Message::tool_results(tool_results));
    }
}

async fn execute_tool(registry: &Registry, name: &str, params: serde_json::Value) -> ToolResult {
    match registry.get(name).await {
        Some(tool) => match tool.execute(params).await {
            Ok(result) => result,
            Err(e) => ToolResult {
                content: format!("Error executing {}: {}", name, e),
                is_error: true,
                metadata: None,
            },
        },
        None => ToolResult {
            content: format!("Tool not found: {}", name),
            is_error: true,
            metadata: None,
        },
    }
}
```

Note: The `StreamEvent::TextDone` variant may not exist — check the actual enum. The code handles unknown variants with `Ok(_) => {}`.

**Step 2: Update agent/mod.rs**

```rust
// ABOUTME: Agent module — LLM provider factory and streaming agent loop.
// ABOUTME: Manages conversation history and tool call dispatch.

pub mod r#loop;
pub mod provider;

pub use provider::*;
```

**Step 3: Verify it compiles**

Run: `cargo build -p soloclaw`
Expected: Compiles (may have warnings)

**Step 4: Commit**

```bash
git add src/agent/
git commit -m "feat: add streaming agent loop with approval integration"
```

---

### Task 13: App Orchestration

**Files:**
- Create: `src/app.rs`
- Modify: `src/main.rs` (full rewrite)

The App struct wires everything together: initializes terminal, spawns agent loop, runs event loop.

**Step 1: Create app.rs**

```rust
// ABOUTME: App orchestrator — wires TUI, agent loop, and approval engine.
// ABOUTME: Manages terminal setup/teardown and the main event loop.

use std::sync::Arc;

use crossterm::event::{self, Event};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::prelude::*;
use tokio::sync::mpsc;

use mux::prelude::*;

use crate::agent;
use crate::approval::{ApprovalEngine, ApprovalDecision};
use crate::config::{self, Config};
use crate::tui::input::{self, InputResult};
use crate::tui::state::*;
use crate::tui::ui;

pub struct App {
    config: Config,
}

impl App {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        // Load .env
        let _ = dotenvy::dotenv();

        // Create LLM client
        let client = agent::create_client(&self.config.llm)?;

        // Create tool registry with built-in tools
        let registry = Registry::new();
        registry.register(BashTool).await;
        registry.register(ReadFileTool).await;
        registry.register(WriteFileTool).await;
        registry.register(ListFilesTool).await;
        registry.register(SearchTool).await;

        // Connect MCP servers
        let mut mcp_clients = Vec::new();
        let mcp_configs = config::load_mcp_configs()?;
        for server_config in mcp_configs {
            let name = server_config.name.clone();
            match McpClient::connect(server_config).await {
                Ok(mut client) => {
                    if let Err(e) = client.initialize().await {
                        eprintln!("Warning: Failed to initialize {}: {}", name, e);
                        continue;
                    }
                    let client = Arc::new(client);
                    match registry.merge_mcp(client.clone(), Some(&name)).await {
                        Ok(count) => {
                            eprintln!("Connected to {} ({} tools)", name, count);
                            mcp_clients.push(client);
                        }
                        Err(e) => eprintln!("Warning: Failed to list tools from {}: {}", name, e),
                    }
                }
                Err(e) => eprintln!("Warning: Failed to connect to {}: {}", name, e),
            }
        }

        let tool_count = registry.list().await.len();

        // Create approval engine
        let approvals_path = Config::approvals_path();
        let engine = Arc::new(ApprovalEngine::new(approvals_path)?);

        // Create channels
        let (user_tx, user_rx) = mpsc::channel::<UserEvent>(32);
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(256);

        // Spawn agent loop
        let agent_client = client.clone();
        let agent_registry = registry.clone();
        let agent_engine = engine.clone();
        let model = self.config.llm.model.clone();
        let max_tokens = self.config.llm.max_tokens;

        tokio::spawn(async move {
            agent::r#loop::run_agent_loop(
                agent_client,
                agent_registry,
                agent_engine,
                model,
                max_tokens,
                user_rx,
                agent_tx,
            )
            .await;
        });

        // Setup terminal
        terminal::enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Setup panic hook to restore terminal
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
            original_hook(panic);
        }));

        // Initialize TUI state
        let mut state = TuiState::new(self.config.llm.model.clone(), tool_count);

        // Main event loop
        let result = self.event_loop(&mut terminal, &mut state, &user_tx, &mut agent_rx).await;

        // Cleanup terminal
        terminal::disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

        // Cleanup MCP clients
        for client in mcp_clients {
            let _ = client.shutdown().await;
        }

        result
    }

    async fn event_loop(
        &self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
        state: &mut TuiState,
        user_tx: &mpsc::Sender<UserEvent>,
        agent_rx: &mut mpsc::Receiver<AgentEvent>,
    ) -> anyhow::Result<()> {
        loop {
            // Render
            terminal.draw(|frame| ui::render(frame, state))?;

            // Poll for events with a short timeout so we can check agent events
            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match input::handle_key(state, key) {
                        InputResult::Quit => {
                            let _ = user_tx.send(UserEvent::Quit).await;
                            return Ok(());
                        }
                        InputResult::Send(text) => {
                            state.push_message(ChatMessageKind::User, text.clone());
                            state.streaming = true;
                            let _ = user_tx.send(UserEvent::Message(text)).await;
                        }
                        InputResult::Approval(_decision) => {
                            // Decision already sent via oneshot in handle_key
                        }
                        InputResult::None => {}
                    }
                }
            }

            // Drain agent events
            while let Ok(event) = agent_rx.try_recv() {
                match event {
                    AgentEvent::TextDelta(text) => {
                        state.append_to_last_assistant(&text);
                    }
                    AgentEvent::TextDone => {}
                    AgentEvent::ToolCallStarted {
                        tool_name,
                        params_summary,
                    } => {
                        state.push_message(
                            ChatMessageKind::ToolCall {
                                tool_name,
                                status: ToolCallStatus::Pending,
                            },
                            params_summary,
                        );
                    }
                    AgentEvent::ToolCallApproved { tool_name } => {
                        // Update the last tool call status
                        update_last_tool_status(state, &tool_name, ToolCallStatus::Allowed);
                    }
                    AgentEvent::ToolCallDenied { tool_name, reason } => {
                        update_last_tool_status(state, &tool_name, ToolCallStatus::Denied);
                    }
                    AgentEvent::ToolCallNeedsApproval {
                        description,
                        pattern,
                        tool_name,
                        responder,
                    } => {
                        state.pending_approval = Some(PendingApproval {
                            description,
                            pattern,
                            tool_name,
                            selected: 0,
                            responder: Some(responder),
                        });
                    }
                    AgentEvent::ToolResult {
                        tool_name,
                        content,
                        is_error,
                    } => {
                        state.push_message(
                            ChatMessageKind::ToolResult { is_error },
                            content,
                        );
                    }
                    AgentEvent::Error(msg) => {
                        state.push_message(ChatMessageKind::System, msg);
                        state.streaming = false;
                    }
                    AgentEvent::Done => {
                        state.streaming = false;
                    }
                }
            }
        }
    }
}

fn update_last_tool_status(state: &mut TuiState, tool_name: &str, status: ToolCallStatus) {
    for msg in state.messages.iter_mut().rev() {
        if let ChatMessageKind::ToolCall {
            tool_name: ref name,
            status: ref mut s,
        } = msg.kind
        {
            if name == tool_name && *s == ToolCallStatus::Pending {
                *s = status;
                break;
            }
        }
    }
}
```

**Step 2: Rewrite main.rs with CLI parsing**

```rust
// ABOUTME: Entry point for soloclaw — a TUI agent with layered tool approval.
// ABOUTME: Parses CLI args, loads config, and launches the app.

mod agent;
mod app;
mod approval;
mod config;
mod tui;

use clap::Parser;

#[derive(Parser)]
#[command(name = "soloclaw", about = "TUI agent with layered tool approval")]
struct Cli {
    /// LLM provider (anthropic, openai, gemini, openrouter, ollama)
    #[arg(long)]
    provider: Option<String>,

    /// Model name
    #[arg(long)]
    model: Option<String>,

    /// Default security level (deny, allowlist, full)
    #[arg(long)]
    security: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let mut config = config::Config::load()?;

    // Apply CLI overrides
    if let Some(provider) = cli.provider {
        config.llm.provider = provider;
    }
    if let Some(model) = cli.model {
        config.llm.model = model;
    }
    if let Some(security) = cli.security {
        config.approval.security = security;
    }

    let app = app::App::new(config);
    app.run().await
}
```

**Step 3: Verify it compiles**

Run: `cargo build -p soloclaw`
Expected: Compiles (fix any issues with mux-rs API mismatches)

**Step 4: Commit**

```bash
git add src/app.rs src/main.rs
git commit -m "feat: add app orchestrator and CLI — wires TUI, agent loop, and approval"
```

---

### Task 14: Integration Test — Approval Engine End-to-End

**Files:**
- Create: `tests/approval_integration.rs`

**Step 1: Write integration test**

```rust
// ABOUTME: Integration tests for the approval engine.
// ABOUTME: Tests the full flow: policy + allowlist + analysis + persistence.

use soloclaw::approval::*;
use std::path::PathBuf;

#[test]
fn full_approval_flow_bash_safe_command() {
    let engine = ApprovalEngine::with_approvals(
        ApprovalsFile::default(),
        PathBuf::from("/tmp/test_approval_integration.json"),
    );

    // Safe command should auto-approve
    let info = ToolCallInfo {
        tool_name: "bash".to_string(),
        params: serde_json::json!({"command": "grep -r 'TODO' src/"}),
    };

    assert!(matches!(engine.check(&info), EngineOutcome::Allowed));
}

#[test]
fn full_approval_flow_bash_unsafe_then_allow_always() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approvals.json");

    let engine = ApprovalEngine::with_approvals(ApprovalsFile::default(), path.clone());

    // Unsafe command should need approval
    let info = ToolCallInfo {
        tool_name: "bash".to_string(),
        params: serde_json::json!({"command": "cargo build"}),
    };

    let outcome = engine.check(&info);
    let pattern = match &outcome {
        EngineOutcome::NeedsApproval { pattern, .. } => pattern.clone(),
        other => panic!("Expected NeedsApproval, got {:?}", std::mem::discriminant(other)),
    };

    // User approves always
    engine.resolve("bash", pattern.as_deref(), ApprovalDecision::AllowAlways);

    // Same command should now auto-approve
    let outcome2 = engine.check(&info);
    assert!(
        matches!(outcome2, EngineOutcome::Allowed),
        "Expected Allowed after AllowAlways"
    );
}

#[test]
fn deny_security_blocks_everything() {
    let mut approvals = ApprovalsFile::default();
    approvals.tools.insert(
        "*".to_string(),
        ToolApprovalConfig {
            security: ToolSecurity {
                security: SecurityLevel::Deny,
                ask: AskMode::Off,
                ask_fallback: AskFallback::Deny,
            },
            allowlist: Vec::new(),
        },
    );

    let engine = ApprovalEngine::with_approvals(
        approvals,
        PathBuf::from("/tmp/test_deny.json"),
    );

    let info = ToolCallInfo {
        tool_name: "read_file".to_string(),
        params: serde_json::json!({"path": "/etc/passwd"}),
    };

    assert!(matches!(engine.check(&info), EngineOutcome::Denied { .. }));
}
```

Note: This requires making the approval module public in `src/main.rs` (add `pub` to `mod approval`). Make all relevant modules `pub` for integration test access.

**Step 2: Run integration tests**

Run: `cargo test --test approval_integration`
Expected: 3 tests pass

**Step 3: Commit**

```bash
git add tests/ src/main.rs
git commit -m "test: add approval engine integration tests"
```

---

### Task 15: E2E Test — TUI Rendering

**Files:**
- Create: `tests/tui_rendering.rs`

**Step 1: Write E2E test using ratatui TestBackend**

```rust
// ABOUTME: E2E tests for TUI rendering using ratatui's TestBackend.
// ABOUTME: Verifies the TUI renders chat messages, status bar, and approval prompts.

use ratatui::prelude::*;
use ratatui::Terminal;

use soloclaw::tui::state::*;
use soloclaw::tui::ui;

fn test_terminal() -> Terminal<ratatui::backend::TestBackend> {
    let backend = ratatui::backend::TestBackend::new(80, 24);
    Terminal::new(backend).unwrap()
}

#[test]
fn renders_empty_state() {
    let mut terminal = test_terminal();
    let state = TuiState::new("test-model".to_string(), 5);

    terminal
        .draw(|frame| ui::render(frame, &state))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    // Verify header contains "soloclaw"
    let header_line: String = (0..80).map(|x| buffer.cell((x, 0)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
    assert!(header_line.contains("soloclaw"));
    assert!(header_line.contains("test-model"));
}

#[test]
fn renders_user_message() {
    let mut terminal = test_terminal();
    let mut state = TuiState::new("m".to_string(), 0);
    state.push_message(ChatMessageKind::User, "Hello agent!".to_string());

    terminal
        .draw(|frame| ui::render(frame, &state))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let all_text: String = (0..24)
        .flat_map(|y| (0..80).map(move |x| buffer.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')))
        .collect();
    assert!(all_text.contains("You:"));
    assert!(all_text.contains("Hello agent!"));
}

#[test]
fn renders_status_bar() {
    let mut terminal = test_terminal();
    let mut state = TuiState::new("claude-sonnet".to_string(), 12);
    state.total_tokens = 1500;

    terminal
        .draw(|frame| ui::render(frame, &state))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    // Status bar is the last line
    let status_line: String = (0..80).map(|x| buffer.cell((x, 23)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
    assert!(status_line.contains("1.5k"));
    assert!(status_line.contains("12"));
}
```

**Step 2: Run E2E tests**

Run: `cargo test --test tui_rendering`
Expected: 3 tests pass

**Step 3: Commit**

```bash
git add tests/tui_rendering.rs
git commit -m "test: add E2E TUI rendering tests with TestBackend"
```

---

## Summary

| Task | Description | Key Files | Tests |
|------|-------------|-----------|-------|
| 1 | Project scaffold | Cargo.toml, main.rs | compile check |
| 2 | Approval types | approval/types.rs | 4 unit |
| 3 | Approval policy | approval/policy.rs | 9 unit |
| 4 | Persistent allowlist | approval/allowlist.rs | 8 unit |
| 5 | Command analysis | approval/analysis.rs | 10 unit |
| 6 | Approval engine | approval/engine.rs | 6 unit |
| 7 | Config loading | config.rs | 3 unit |
| 8 | Provider factory | agent/provider.rs | 1 unit |
| 9 | TUI state & events | tui/state.rs | 6 unit |
| 10 | TUI widgets | tui/widgets/*.rs | 5 unit |
| 11 | TUI rendering & input | tui/ui.rs, tui/input.rs | 6 unit |
| 12 | Agent loop | agent/loop.rs | compile check |
| 13 | App orchestration | app.rs, main.rs | compile check |
| 14 | Integration tests | tests/approval_integration.rs | 3 integration |
| 15 | E2E TUI tests | tests/tui_rendering.rs | 3 E2E |

**Total: 15 tasks, ~64 tests, 15 commits**
