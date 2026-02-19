# Boba TUI Migration Design

## Overview

Migrate soloclaw's TUI from raw ratatui + manual event loop to the boba framework (Elm Architecture on ratatui). The agent loop, approval engine, session persistence, compaction, and all backend code stay untouched. Only the TUI layer changes.

## Approach

**Boba core + selective widget adoption:**
- Use boba's `Model`/`Command`/`Subscription` architecture
- Adopt `TextArea` for input (undo/redo, selection, word ops)
- Adopt `StatusBar` for the bottom bar
- Keep custom rendering for chat, approval, and question widgets

## Architecture

### Model Struct

Replaces `TuiState` + the manual event loop in `app.rs`:

```rust
struct ClawApp {
    // Boba component
    input: TextArea,

    // Chat state
    messages: Vec<ChatMessage>,
    scroll_offset: u16,
    streaming: bool,
    queued_message: Option<String>,

    // Modal prompts
    pending_approval: Option<PendingApproval>,
    pending_question: Option<PendingQuestion>,

    // Session metadata
    model_name: String,
    tool_count: usize,
    total_tokens: u64,
    context_window: u64,
    context_used: u64,
    session_start: Instant,
    workspace_dir: String,

    // Channels
    user_tx: mpsc::Sender<UserEvent>,
    agent_rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<AgentEvent>>>>,
}
```

### Message Enum

```rust
enum Msg {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    Agent(AgentEvent),
    Input(text_area::Message),
    MessageSent,
}
```

### Flags

```rust
struct Flags {
    user_tx: mpsc::Sender<UserEvent>,
    agent_rx: mpsc::Receiver<AgentEvent>,
    model_name: String,
    tool_count: usize,
    context_window: u64,
    workspace_dir: String,
    replay_messages: Vec<ChatMessage>,
    startup_message: String,
}
```

### Subscriptions

Two event sources replace the manual poll loop:

1. **Terminal events** — boba's built-in `terminal_events()` maps key, mouse, paste, resize to `Msg` variants.

2. **Agent events** — custom `SubscriptionSource` wrapping the `mpsc::Receiver<AgentEvent>`. The receiver is stored in `Arc<Mutex<Option<Receiver>>>`. On first subscription start, the stream takes the receiver. Subsequent `subscriptions()` calls return the same ID so the runtime keeps it alive without re-taking.

### Update Logic

Mode-aware key routing:

```
Key event arrives
  → pending_approval?  → approval handler (Left/Right nav, 1/2/3, Enter)
  → pending_question?  → question handler (free-text or multichoice)
  → streaming?         → intercept Enter (queue), forward rest to TextArea
  → normal             → intercept Enter (send), Esc (quit),
                         PageUp/Down (scroll), context-aware Up/Down,
                         forward rest to TextArea
```

Agent events map 1:1 from current `handle_agent_event()`.

Sending messages to the agent uses `Command::perform` with the cloned `user_tx`.

Oneshot channels for approval/question responses are sent synchronously from `update()`.

### View Layout

```
┌─────────────────────────────────────┐
│ Header (1 line)                     │  custom render
├─────────────────────────────────────┤
│ Chat Area (flex)                    │  custom render (chat.rs logic)
├─────────────────────────────────────┤
│ Prompt Area (0-4 lines, dynamic)    │  custom render (approval/question)
├─────────────────────────────────────┤
│ Input Area (3-8 lines, dynamic)     │  boba TextArea.view()
├─────────────────────────────────────┤
│ Status Bar (1 line)                 │  boba StatusBar
└─────────────────────────────────────┘
```

### Startup Flow

```
main.rs
  → Config::load()
  → create LLM client, registry, approval engine
  → create channels (user_tx/rx, agent_tx/rx)
  → spawn agent loop
  → build Flags
  → boba::run_with::<ClawApp>(flags, ProgramOptions { ... })
  → print farewell screen
  → cleanup (agent shutdown, MCP shutdown)
```

`ProgramOptions` replaces manual terminal setup:
```rust
ProgramOptions {
    fps: 30,
    alt_screen: true,
    mouse_mode: Some(MouseMode::CellMotion),
    bracketed_paste: true,
    catch_panics: true,
    ..Default::default()
}
```

## Widget Adoption

| Widget | Source | Reason |
|---|---|---|
| TextArea (input) | boba | Free undo/redo, selection, word ops |
| StatusBar | boba | Clean 3-section layout API |
| Chat messages | custom | Emoji prefixes, tool call status, result truncation |
| Approval prompt | custom | Inline widget, not modal overlay |
| Question prompt | custom | Inline widget with multichoice |
| Header | custom | Simple 1-line render |

## Files Changed

### Rewritten
- `src/app.rs` — Setup + `boba::run_with()`, no manual event loop
- `src/tui/state.rs` — `ClawApp` Model, `Msg`, `Flags`, `AgentEventSource`
- `src/tui/input.rs` — Dissolves into `ClawApp::update()` match arms
- `src/tui/ui.rs` — Becomes `ClawApp::view()`, keeps custom render helpers
- `src/tui/mod.rs` — Updated module structure

### Adapted
- `src/tui/widgets/chat.rs` — Render functions stay, called from `view()`
- `src/tui/widgets/approval.rs` — Render functions stay
- `src/tui/widgets/question.rs` — Render functions stay
- `src/tui/widgets/status.rs` — Replaced by boba `StatusBar` with custom content

### Unchanged
- `src/agent/` — Agent loop, compaction, provider
- `src/approval/` — Policy, analysis, allowlist, engine
- `src/session/` — Logger, persistence
- `src/tools/` — ask_user tool
- `src/config.rs` — Config loading
- `src/prompt.rs` — System prompt builder
- `src/main.rs` — Minor: calls `boba::run_with` via updated `App::run`

### Dependencies
- Add: `boba = { path = "../boba" }`
- Keep: `ratatui`, `crossterm` (used in custom view rendering)
- Add: `tokio-stream` (for `ReceiverStream` in agent subscription)

## Testing

- **Headless model tests**: boba `TestProgram` for update logic
- **Existing tests**: Agent loop, approval, session, compaction — unchanged
- **Custom render tests**: Chat, approval, question widget rendering
- **Integration**: Full message flow through subscriptions → update → view

## Key Design Decisions

1. **Agent channel as SubscriptionSource** — wraps mpsc receiver, auto-managed by boba's subscription diffing
2. **TextArea with intercepted keys** — Enter sends (not newline), Shift+Enter forwarded to TextArea for newline, context-aware Up/Down checks cursor_row before forwarding
3. **Custom chat rendering** — boba's Chat widget doesn't support our emoji prefix/tool status styling
4. **Inline prompts over modals** — approval and question prompts stay inline, matching terminal agent UX conventions
5. **ProgramOptions for terminal** — eliminates manual enable_raw_mode/EnterAlternateScreen/panic hook code
