// ABOUTME: Shell command analysis — pipeline parsing, safe-bin detection, and PATH resolution.
// ABOUTME: Splits commands on shell operators, resolves executables, and determines safety.

use std::path::{Path, PathBuf};

/// A single segment of a parsed command (one executable with its arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSegment {
    /// The executable name or path.
    pub executable: String,
    /// Arguments passed to the executable.
    pub args: Vec<String>,
    /// Whether this segment only processes stdin (i.e. is a piped-to command).
    pub stdin_only: bool,
}

/// The result of analyzing a shell command string.
#[derive(Debug, Clone)]
pub struct AnalysisResult {
    /// All parsed command segments across pipes and chains.
    pub segments: Vec<CommandSegment>,
    /// The resolved absolute path of the first executable, if found.
    pub resolved_path: Option<PathBuf>,
    /// Whether all segments use safe stdin-only binaries.
    pub safe: bool,
}

/// Binaries considered safe because they only read/transform stdin or produce output.
pub const SAFE_BINS: &[&str] = &[
    "awk", "base64", "cat", "column", "cut", "diff", "echo", "env", "expand", "expr", "false",
    "fmt", "fold", "grep", "head", "jq", "less", "more", "nl", "od", "paste", "printf", "rev",
    "sed", "seq", "shuf", "sort", "strings", "tac", "tail", "tee", "tr", "true", "tsort", "uniq",
    "wc", "xargs", "yes",
];

/// Check if a binary name (possibly an absolute path) is in the safe list.
pub fn is_safe_bin(name: &str) -> bool {
    let basename = Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);
    SAFE_BINS.contains(&basename)
}

/// Resolve an executable name to an absolute path by searching PATH.
///
/// Returns None if the name is already absolute but doesn't exist,
/// or if it can't be found in any PATH directory.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);

    // Already absolute — just check existence.
    if path.is_absolute() {
        if path.exists() {
            return Some(path.to_path_buf());
        }
        return None;
    }

    // Search PATH entries.
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = Path::new(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Quote-aware word splitting that respects single quotes, double quotes, and backslash escaping.
fn shell_words(input: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            if c == '\'' {
                in_single_quote = false;
            } else {
                current.push(c);
            }
        } else if in_double_quote {
            if c == '\\' {
                // Inside double quotes, backslash escapes the next char.
                if let Some(&next) = chars.peek() {
                    chars.next();
                    current.push(next);
                }
            } else if c == '"' {
                in_double_quote = false;
            } else {
                current.push(c);
            }
        } else if c == '\\' {
            // Backslash escaping outside quotes.
            if let Some(&next) = chars.peek() {
                chars.next();
                current.push(next);
            }
        } else if c == '\'' {
            in_single_quote = true;
        } else if c == '"' {
            in_double_quote = true;
        } else if c.is_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Parse a shell command string into pipeline segments.
///
/// Splits on chain operators (&&, ||, ;) to get independent commands,
/// then splits each on | to get piped segments.
pub fn parse_pipeline(command: &str) -> Vec<CommandSegment> {
    let mut segments = Vec::new();

    // Split on chain operators: &&, ||, ;
    // We need to split on the string literals, being careful about ordering
    // (check && and || before single & or |).
    let chains = split_on_chain_operators(command);

    for chain in &chains {
        let chain = chain.trim();
        if chain.is_empty() {
            continue;
        }

        // Split each chain on pipe |.
        let pipe_parts = split_on_pipe(chain);

        for (i, part) in pipe_parts.iter().enumerate() {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let words = shell_words(part);
            if words.is_empty() {
                continue;
            }
            segments.push(CommandSegment {
                executable: words[0].clone(),
                args: words[1..].to_vec(),
                stdin_only: i > 0,
            });
        }
    }

    segments
}

/// Split a command string on the chain operators &&, ||, and ;.
fn split_on_chain_operators(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            current.push(c);
            if c == '\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            current.push(c);
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    current.push(next);
                }
            } else if c == '"' {
                in_double_quote = false;
            }
        } else if c == '\'' {
            in_single_quote = true;
            current.push(c);
        } else if c == '"' {
            in_double_quote = true;
            current.push(c);
        } else if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        } else if c == '|' {
            if chars.peek() == Some(&'|') {
                chars.next();
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        } else if c == ';' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Split a single chain segment on the pipe operator |.
/// Respects quotes so that | inside quotes is not treated as a pipe.
fn split_on_pipe(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            current.push(c);
            if c == '\'' {
                in_single_quote = false;
            }
        } else if in_double_quote {
            current.push(c);
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    chars.next();
                    current.push(next);
                }
            } else if c == '"' {
                in_double_quote = false;
            }
        } else if c == '\'' {
            in_single_quote = true;
            current.push(c);
        } else if c == '"' {
            in_double_quote = true;
            current.push(c);
        } else if c == '|' {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Analyze a shell command string: parse it, resolve the first executable, and check safety.
pub fn analyze_command(command: &str) -> AnalysisResult {
    let segments = parse_pipeline(command);

    let resolved_path = segments
        .first()
        .and_then(|seg| resolve_executable(&seg.executable));

    let safe = !segments.is_empty() && segments.iter().all(|seg| is_safe_bin(&seg.executable));

    AnalysisResult {
        segments,
        resolved_path,
        safe,
    }
}

/// Determine the allowlist pattern for a command analysis result.
///
/// Prefers the resolved absolute path; falls back to the executable name.
pub fn allowlist_pattern(analysis: &AnalysisResult) -> Option<String> {
    if let Some(ref resolved) = analysis.resolved_path {
        return Some(resolved.to_string_lossy().into_owned());
    }
    analysis
        .segments
        .first()
        .map(|seg| seg.executable.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_bin_recognizes_common_tools() {
        assert!(is_safe_bin("cat"));
        assert!(is_safe_bin("grep"));
        assert!(is_safe_bin("sort"));
        assert!(is_safe_bin("wc"));
        assert!(is_safe_bin("jq"));
        assert!(!is_safe_bin("rm"));
        assert!(!is_safe_bin("bash"));
        assert!(!is_safe_bin("python"));
    }

    #[test]
    fn is_safe_bin_handles_absolute_paths() {
        assert!(is_safe_bin("/usr/bin/cat"));
        assert!(is_safe_bin("/usr/bin/grep"));
        assert!(!is_safe_bin("/usr/bin/rm"));
    }

    #[test]
    fn parse_simple_command() {
        let segments = parse_pipeline("ls -la /tmp");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].executable, "ls");
        assert_eq!(segments[0].args, vec!["-la", "/tmp"]);
        assert!(!segments[0].stdin_only);
    }

    #[test]
    fn parse_pipeline_segments() {
        let segments = parse_pipeline("cat file.txt | grep pattern | sort");
        assert_eq!(segments.len(), 3);

        assert_eq!(segments[0].executable, "cat");
        assert_eq!(segments[0].args, vec!["file.txt"]);
        assert!(!segments[0].stdin_only);

        assert_eq!(segments[1].executable, "grep");
        assert_eq!(segments[1].args, vec!["pattern"]);
        assert!(segments[1].stdin_only);

        assert_eq!(segments[2].executable, "sort");
        assert!(segments[2].args.is_empty());
        assert!(segments[2].stdin_only);
    }

    #[test]
    fn parse_chained_commands() {
        let segments = parse_pipeline("echo hello && cat file ; wc -l");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].executable, "echo");
        assert_eq!(segments[1].executable, "cat");
        assert_eq!(segments[2].executable, "wc");
        // Each chained command starts fresh (not stdin_only).
        assert!(!segments[0].stdin_only);
        assert!(!segments[1].stdin_only);
        assert!(!segments[2].stdin_only);
    }

    #[test]
    fn parse_quoted_args() {
        let segments = parse_pipeline(r#"echo "hello world" 'foo bar'"#);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].executable, "echo");
        assert_eq!(segments[0].args, vec!["hello world", "foo bar"]);
    }

    #[test]
    fn analyze_safe_pipeline() {
        let result = analyze_command("cat file.txt | grep pattern | sort | uniq");
        assert!(result.safe);
        assert_eq!(result.segments.len(), 4);
    }

    #[test]
    fn analyze_unsafe_command() {
        let result = analyze_command("rm -rf /");
        assert!(!result.safe);
        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.segments[0].executable, "rm");
    }

    #[test]
    fn analyze_mixed_pipeline_unsafe() {
        let result = analyze_command("cat file.txt | python script.py | sort");
        assert!(!result.safe);
        assert_eq!(result.segments.len(), 3);
    }

    #[test]
    fn allowlist_pattern_uses_resolved_path() {
        // Create a result with a resolved path.
        let result = AnalysisResult {
            segments: vec![CommandSegment {
                executable: "cat".to_string(),
                args: vec![],
                stdin_only: false,
            }],
            resolved_path: Some(PathBuf::from("/usr/bin/cat")),
            safe: true,
        };
        assert_eq!(allowlist_pattern(&result), Some("/usr/bin/cat".to_string()));
    }

    #[test]
    fn allowlist_pattern_falls_back_to_name() {
        let result = AnalysisResult {
            segments: vec![CommandSegment {
                executable: "my_tool".to_string(),
                args: vec![],
                stdin_only: false,
            }],
            resolved_path: None,
            safe: false,
        };
        assert_eq!(allowlist_pattern(&result), Some("my_tool".to_string()));
    }
}
