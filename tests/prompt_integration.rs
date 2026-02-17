// ABOUTME: Integration tests for the dynamic system prompt builder.
// ABOUTME: Verifies prompt assembly from runtime parameters and context files.

use std::collections::HashMap;

use simpleclaw::prompt::{build_system_prompt, load_context_files, ContextFile, SystemPromptParams};

fn base_params() -> SystemPromptParams {
    SystemPromptParams {
        tool_names: vec![
            "bash".to_string(),
            "read_file".to_string(),
            "write_file".to_string(),
            "list_files".to_string(),
            "search".to_string(),
        ],
        tool_summaries: {
            let mut m = HashMap::new();
            m.insert(
                "bash".to_string(),
                "Execute a bash command and return its output.".to_string(),
            );
            m.insert(
                "read_file".to_string(),
                "Read the contents of a file.".to_string(),
            );
            m.insert(
                "write_file".to_string(),
                "Write content to a file.".to_string(),
            );
            m.insert(
                "list_files".to_string(),
                "List files in a directory.".to_string(),
            );
            m.insert(
                "search".to_string(),
                "Search for a pattern in files.".to_string(),
            );
            m
        },
        workspace_dir: "/home/user/project".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        shell: "/bin/bash".to_string(),
        model: "claude-sonnet-4-20250514".to_string(),
        context_files: vec![],
    }
}

#[test]
fn full_prompt_has_all_runtime_sections() {
    let prompt = build_system_prompt(&base_params());

    assert!(prompt.contains("SimpleClaw"), "missing identity");
    assert!(prompt.contains("## Tooling"), "missing tooling section");
    assert!(
        prompt.contains("- bash: Execute a bash command"),
        "missing bash tool"
    );
    assert!(
        prompt.contains("## Tool Call Style"),
        "missing tool call style"
    );
    assert!(prompt.contains("## Safety"), "missing safety");
    assert!(prompt.contains("## Workspace"), "missing workspace");
    assert!(
        prompt.contains("/home/user/project"),
        "missing workspace dir"
    );
    assert!(
        prompt.contains("## Current Date & Time"),
        "missing datetime"
    );
    assert!(prompt.contains("## Runtime"), "missing runtime");
    assert!(prompt.contains("os=linux (x86_64)"), "missing os info");
    assert!(
        prompt.contains("model=claude-sonnet-4-20250514"),
        "missing model"
    );
}

#[test]
fn prompt_without_context_files_has_no_project_context() {
    let prompt = build_system_prompt(&base_params());
    assert!(!prompt.contains("# Project Context"));
}

#[test]
fn prompt_with_soul_file_embodies_persona() {
    let mut params = base_params();
    params.context_files = vec![ContextFile {
        path: "SOUL.md".to_string(),
        content: "Be a friendly pirate who loves Rust.".to_string(),
    }];
    let prompt = build_system_prompt(&params);
    assert!(prompt.contains("# Project Context"));
    assert!(prompt.contains("embody its persona"));
    assert!(prompt.contains("friendly pirate"));
}

#[test]
fn prompt_with_multiple_context_files() {
    let mut params = base_params();
    params.context_files = vec![
        ContextFile {
            path: ".simpleclaw.md".to_string(),
            content: "This project uses React.".to_string(),
        },
        ContextFile {
            path: "AGENTS.md".to_string(),
            content: "Follow TDD.".to_string(),
        },
    ];
    let prompt = build_system_prompt(&params);
    assert!(prompt.contains("## .simpleclaw.md"));
    assert!(prompt.contains("This project uses React."));
    assert!(prompt.contains("## AGENTS.md"));
    assert!(prompt.contains("Follow TDD."));
}

#[test]
fn all_five_builtin_tools_listed() {
    let prompt = build_system_prompt(&base_params());
    for tool in &["bash", "read_file", "write_file", "list_files", "search"] {
        assert!(
            prompt.contains(&format!("- {}:", tool)),
            "missing tool: {}",
            tool
        );
    }
}

#[test]
fn load_context_files_returns_empty_for_nonexistent_dir() {
    let files = load_context_files("/nonexistent/path/that/does/not/exist");
    assert!(files.is_empty());
}

#[test]
fn load_context_files_finds_files_in_workspace() {
    let dir = std::env::temp_dir().join("simpleclaw-integration-ctx-v2");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("SOUL.md"), "Be awesome.").unwrap();
    std::fs::write(dir.join(".simpleclaw.md"), "Project notes.").unwrap();

    let files = load_context_files(dir.to_str().unwrap());
    assert!(files.iter().any(|f| f.path == "SOUL.md"));
    assert!(files.iter().any(|f| f.path == ".simpleclaw.md"));

    let _ = std::fs::remove_file(dir.join("SOUL.md"));
    let _ = std::fs::remove_file(dir.join(".simpleclaw.md"));
}

#[test]
fn section_order_matches_openclaw() {
    let prompt = build_system_prompt(&base_params());

    let identity_pos = prompt.find("SimpleClaw").unwrap();
    let tooling_pos = prompt.find("## Tooling").unwrap();
    let style_pos = prompt.find("## Tool Call Style").unwrap();
    let safety_pos = prompt.find("## Safety").unwrap();
    let workspace_pos = prompt.find("## Workspace").unwrap();
    let datetime_pos = prompt.find("## Current Date & Time").unwrap();
    let runtime_pos = prompt.find("## Runtime").unwrap();

    assert!(identity_pos < tooling_pos, "identity before tooling");
    assert!(tooling_pos < style_pos, "tooling before style");
    assert!(style_pos < safety_pos, "style before safety");
    assert!(safety_pos < workspace_pos, "safety before workspace");
    assert!(workspace_pos < datetime_pos, "workspace before datetime");
    assert!(datetime_pos < runtime_pos, "datetime before runtime");
}
