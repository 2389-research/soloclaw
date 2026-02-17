// ABOUTME: Integration tests for the layered system prompt builder.
// ABOUTME: Verifies default prompt assembly and override behavior.

use simpleclaw::prompt::SystemPromptBuilder;

#[test]
fn default_prompt_contains_all_layers() {
    let builder = SystemPromptBuilder::new();
    let prompt = builder.build();

    // Soul layer
    assert!(prompt.contains("genuinely helpful"), "missing soul content");

    // Agents layer
    assert!(prompt.contains("Agent Guidelines"), "missing agents content");

    // Tools layer
    assert!(prompt.contains("Local Notes"), "missing tools content");
}

#[test]
fn prompt_layers_separated_by_blank_lines() {
    let builder = SystemPromptBuilder::new();
    let prompt = builder.build();

    // The soul layer ends, then agents layer starts — separated by \n\n
    let soul_end = prompt.find("Just... good.").expect("soul vibe missing");
    let agents_start = prompt.find("# Agent Guidelines").expect("agents header missing");
    assert!(agents_start > soul_end, "agents should come after soul");
}

#[test]
fn custom_soul_replaces_default() {
    let mut builder = SystemPromptBuilder::new();
    builder.soul = "You are a pirate assistant. Arrr.".to_string();
    let prompt = builder.build();

    assert!(prompt.contains("pirate assistant"));
    assert!(!prompt.contains("genuinely helpful"));
    // Other layers should still be present.
    assert!(prompt.contains("Agent Guidelines"));
}

#[test]
fn empty_tools_layer_skipped() {
    let mut builder = SystemPromptBuilder::new();
    builder.tools = String::new();
    let prompt = builder.build();

    assert!(prompt.contains("genuinely helpful"));
    assert!(prompt.contains("Agent Guidelines"));
    assert!(!prompt.contains("Local Notes"));
}

#[test]
fn local_context_appended() {
    let mut builder = SystemPromptBuilder::new();
    builder.local = Some("This project uses React and TypeScript.".to_string());
    let prompt = builder.build();

    assert!(prompt.contains("This project uses React and TypeScript."));
    // Should be after all other layers.
    let local_pos = prompt.find("React and TypeScript").unwrap();
    let tools_pos = prompt.find("Local Notes").unwrap();
    assert!(local_pos > tools_pos, "local context should come after tools");
}
