# Agent Guidelines

## Tool Usage

You have access to tools for running shell commands, reading files, writing files, listing directory contents, and searching codebases. Use them proactively — read before modifying, search before guessing, verify before claiming.

- **bash**: Run shell commands. Prefer non-destructive commands. For destructive operations (rm, overwriting files, force-pushing), confirm with the user first.
- **read_file**: Read file contents. Always read a file before modifying it.
- **write_file**: Write or overwrite files. Preserve existing style and conventions.
- **list_files**: List directory contents. Use to orient yourself in unfamiliar codebases.
- **search**: Search file contents with patterns. Use to find definitions, usages, and references.

## Coding Style

- Prefer strict typing; avoid dynamic or `any`-style patterns.
- Add brief code comments for tricky or non-obvious logic.
- Keep files concise; aim for under ~500 lines. Split and refactor when it improves clarity.
- Match the existing style of the codebase. Consistency within a project matters more than personal preference.
- Never disable linters, type checkers, or safety checks. Fix root causes instead.
- Avoid "V2" copies of files — extract helpers and refactor.

## Commit & Version Control

- Write concise, action-oriented commit messages (e.g., "add verbose flag to send command").
- Group related changes together; avoid bundling unrelated work.
- Never commit secrets, credentials, or sensitive configuration values.
- Run tests before claiming work is complete.

## Testing

- Run the project's test suite before pushing or claiming changes work.
- Verify behavior in code — do not guess or assume.
- When adding functionality, ensure tests cover the new behavior.

## Security

- Never commit or output real secrets, API keys, tokens, or passwords.
- Use obviously fake placeholders in examples and documentation.
- Be cautious with external network requests — confirm intent before making them.
- Prefer the principle of least privilege in all operations.

## Working With Others

- Focus on your assigned changes; don't touch unrelated code.
- When you see unfamiliar files or changes, investigate before modifying.
- Respond with high-confidence answers only. If unsure, say so and explain what you'd need to verify.
- When answering questions about code, verify in the source — do not guess.

## Problem Solving

- Read source code of relevant dependencies and all related local code before concluding.
- Aim for high-confidence root causes in bug investigations.
- Prefer fixing root causes over applying workarounds.
- When blocked, explain what you tried and what didn't work.
