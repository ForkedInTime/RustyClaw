# RustyClaw

A Rust-native Claude Code CLI. Single binary, no runtime, ~10ms startup.

```bash
curl -fsSL https://raw.githubusercontent.com/ForkedInTime/RustyClaw/main/install.sh | bash
```

![Welcome screen](assets/welcome.png)

---

## Why?

| | Claude Code (npm) | RustyClaw |
|---|---|---|
| Runtime | Node.js / Bun | None (native binary) |
| Startup | ~300ms | ~10ms |
| Memory | ~150MB | ~10MB |
| Binary | ~50MB JS bundle | ~8MB stripped |
| Ollama | No | Built-in |
| Voice TTS | No | Piper integration |
| Sandbox | No | bwrap / firejail / strict |

Same Claude API, same tools, same CLAUDE.md format. Just faster and self-contained.

---

## Features at a Glance

### Interactive Model Picker

Switch between Claude models and local Ollama models mid-session. Tab-complete model names, or browse with `/model`.

![Ollama model picker](assets/ollama-models.png)

### Session Manager

Browse, resume, and delete sessions with `/session`. Previews show what each session was about.

![Session picker](assets/session-picker.png)

### Interactive Help

`/help` opens a two-level menu. Pick a category, then pick a command to run it.

![Help menu](assets/help-menu.png)

### Voice Input & TTS

Record with `Ctrl+R`, transcribe with Whisper, and hear responses spoken back via Piper TTS. Pick your voice model with `/voice model`.

![Voice status](assets/voice-status.png)

### 30+ Tools

Bash, Read, Write, Edit, Glob, Grep, WebFetch, WebSearch, Agent, LSP, Jupyter notebooks, git worktrees, background tasks, cron scheduling, MCP plugins, and more.

### Custom Spinner

Animated `∙ ✦ ✸ ❊ ✺ ❋` glyphs with 260+ themed verbs (*Boss-fighting, Infusing, Cadence-checking...*) and completion stats (*Sprinted for 3m 44s · 1.2k tokens*).

---

## Install

**One-liner (Linux):**
```bash
curl -fsSL https://raw.githubusercontent.com/ForkedInTime/RustyClaw/main/install.sh | bash
```

**From source:**
```bash
git clone https://github.com/ForkedInTime/RustyClaw.git
cd RustyClaw
cargo build --release
./target/release/rustyclaw
```

**Specific version:**
```bash
curl -fsSL https://raw.githubusercontent.com/ForkedInTime/RustyClaw/main/install.sh | bash -s v0.1.0
```

---

## Quick Start

```bash
# 1. Set your API key
echo 'ANTHROPIC_API_KEY=sk-ant-...' >> ~/.env

# 2. Run
rustyclaw

# 3. Try some commands
/help              # interactive command menu
/model              # pick a model
/session            # browse sessions
/voice model        # pick a TTS voice
```

`.env` files are auto-loaded from `$CWD/.env`, `~/.env`, or `~/.config/rustyclaw/.env`.

---

## Ollama (Local Models)

```bash
ollama pull dolphin3
rustyclaw
```
```
/model ollama:dolphin3     # switch to local model
/model                     # interactive picker (Claude + Ollama)
/model default             # back to Claude
```

Models that don't support tool use get automatic text-only fallback.

---

## Voice

```bash
# Install piper for TTS
pip install piper-tts
mkdir -p ~/.local/share/piper && cd ~/.local/share/piper
wget https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/high/en_US-lessac-high.onnx
wget https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/high/en_US-lessac-high.onnx.json
```

```
/voice enable       # voice input (Ctrl+R to record)
/voice speak on     # TTS responses
/voice model        # pick a voice (plays preview)
/doctor             # check everything is set up
```

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | Newline |
| `Esc` | Cancel request / stop TTS / close overlay |
| `Ctrl+S` | Stop TTS |
| `Ctrl+R` | Voice record toggle |
| `?` | Shortcuts overlay |
| `PgUp/PgDn` | Scroll chat |
| `Tab` | Autocomplete commands / models / history |
| `Shift+click` | Select text |
| `Ctrl+Shift+C` | Copy selection |

---

## Architecture

```
src/
├── main.rs           # Entry point, CLI args, .env auto-load
├── api/              # Anthropic + Ollama backends (streaming SSE)
├── tui/              # ratatui UI (inline viewport, no alt screen)
├── tools/            # 30+ tools (Bash, Read, Write, Edit, Glob, Grep, ...)
├── commands/         # 60+ slash commands
├── mcp/              # MCP plugin client
├── session/          # Save/resume/search/export sessions
├── voice.rs          # Recording + Whisper + Piper TTS
├── sandbox.rs        # bwrap / firejail / strict
└── config.rs         # Settings, CLAUDE.md injection
```

Built with `tokio`, `ratatui`, `reqwest` (rustls), `clap`, `serde_json`.

---

## License

Derived from [Claude Code](https://github.com/anthropics/claude-code). For personal and educational use.
