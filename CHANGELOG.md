# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **SDK / Headless mode** — `--headless` flag starts an NDJSON stdio server for embedding in editors, CI/CD, scripts, and custom UIs. Full protocol reference in [`sdk/`](sdk/).
- **Phase 1 robustness** — AGENTS.md support, XDG Base Directory compliance, context usage % in status bar, always-show-thinking, spinner style toggle, `/reload` hot-reload.
- **Auto-fix loop** (Phase 2) — After the model edits code, RustyClaw runs
  project-appropriate lint + test commands and, on failure, injects the
  output back as a synthetic user turn for up to `maxRetries` rounds
  (default 3, cap 10). Supports Rust (`cargo clippy` + `cargo test`),
  Node (`npx eslint` + `npm test`), Python (`ruff check` + `pytest`),
  and Go (`go vet` + `go test`). Anti-cheat clause in the feedback
  prompt blocks `#[allow(dead_code)]`-style escapes.
  Configure via `autoFixLoop` in `settings.json`; `autoRollback` still
  works as an alias.

### Changed

- The `auto_rollback` module has been renamed to `autofix` and no longer
  reverts files on failure. On retry-cap, the working tree is left
  as-is; use `/undo` or `git checkout` to revert manually.

## [0.1.0] - 2026-04-07

### Added

- Initial release.
- **Anthropic API backend** — streaming SSE, all Claude models.
- **Ollama backend** — local model discovery, tool-use fallback, model picker.
- **OpenAI-compatible providers** — Groq, OpenRouter, DeepSeek, LM Studio, Together, Mistral, Venice.ai, OpenAI, generic endpoints.
- **30+ tools** — Bash, Read, Write, Edit, Glob, Grep, WebFetch, WebSearch, Agent, LSP, Jupyter, MCP plugins, and more.
- **60+ slash commands** — `/help`, `/model`, `/session`, `/voice`, `/doctor`, `/rag`, `/budget`, and more.
- **RAG indexing** — tree-sitter AST parsing + SQLite FTS5 search across 8 languages.
- **Smart model router** — auto-detect task complexity, route to cheapest capable model.
- **Cost tracking** — real-time token/cost dashboard with budget limits.
- **Voice I/O** — Whisper STT + Piper TTS + XTTS v2 voice cloning.
- **Session management** — save, resume, search, export conversations.
- **Interactive pickers** — model, session, help, voice model selection with previews.
- **Custom spinner** — 260+ themed verbs with animated glyphs and completion stats.
- **Sandboxing** — bwrap / firejail / strict isolation.
- **Inline TUI** — ratatui-based, no alt screen, zero flicker.
- **Cross-compilation** — CI builds x86_64-gnu, aarch64-gnu, x86_64-musl via `cross`.
- **Install script** — one-liner install with version pinning.

[Unreleased]: https://github.com/ForkedInTime/RustyClaw/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ForkedInTime/RustyClaw/releases/tag/v0.1.0
