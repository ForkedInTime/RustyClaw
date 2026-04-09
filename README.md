<p align="center">
  <img src="assets/welcome.png" alt="RustyClaw" width="700">
</p>

<h3 align="center">A Rust-native Claude Code CLI</h3>

<p align="center">
  Single binary · No runtime · ~10ms startup · ~10MB RAM
</p>

<p align="center">
  <a href="https://github.com/ForkedInTime/RustyClaw/releases"><img src="https://img.shields.io/github/v/release/ForkedInTime/RustyClaw?style=flat-square&color=blue" alt="Release"></a>
  <a href="https://github.com/ForkedInTime/RustyClaw/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/ForkedInTime/RustyClaw/ci.yml?style=flat-square&label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-2024_edition-orange?style=flat-square&logo=rust" alt="Rust"></a>
</p>

---

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/ForkedInTime/RustyClaw/main/install.sh | bash
```

<details>
<summary>Other methods</summary>

**From source:**
```bash
git clone https://github.com/ForkedInTime/RustyClaw.git
cd RustyClaw && cargo build --release
./target/release/rustyclaw
```

**Specific version:**
```bash
curl -fsSL https://raw.githubusercontent.com/ForkedInTime/RustyClaw/main/install.sh | bash -s v0.1.0
```
</details>

---

## Why RustyClaw?

| | Claude Code (npm) | RustyClaw |
|---|---|---|
| Runtime | Node.js / Bun | None (native binary) |
| Startup | ~300ms | **~10ms** |
| Memory | ~150MB | **~10MB** |
| Binary | ~50MB JS bundle | **~8MB stripped** |
| Ollama | No | **Built-in** |
| Voice / TTS | No | **XTTS v2 voice cloning** |
| Codebase RAG | No | **tree-sitter + FTS5** |
| Model routing | No | **Auto complexity routing** |
| Cost tracking | No | **Real-time dashboard** |
| Sandbox | No | **bwrap / firejail** |
| SDK / Headless | No | **NDJSON stdio** |

Same Claude API. Same tools. Same CLAUDE.md format. Just faster and self-contained.

---

## Quick Start

```bash
# Set your API key
echo 'ANTHROPIC_API_KEY=sk-ant-...' >> ~/.env

# Run
rustyclaw

# Explore
/help              # interactive command menu
/model             # pick a model (Claude + Ollama)
/doctor            # verify setup
```

`.env` files auto-load from `$CWD/.env`, `~/.env`, or `~/.config/rustyclaw/.env`.

---

## Screenshots

<table>
<tr>
<td width="50%">

**Interactive Help**
![Help menu](assets/help-menu.png)

</td>
<td width="50%">

**Model Picker**
![Model picker](assets/ollama-models.png)

</td>
</tr>
<tr>
<td width="50%">

**Session Manager**
![Session picker](assets/session-picker.png)

</td>
<td width="50%">

**Doctor Diagnostics**
![Doctor](assets/doctor.png)

</td>
</tr>
</table>

---

## Highlights

- **30+ tools** — Bash, Read, Write, Edit, Glob, Grep, WebFetch, Agent, LSP, Jupyter, MCP plugins, and more
- **60+ slash commands** — `/help`, `/model`, `/session`, `/voice`, `/doctor`, `/rag`, `/budget`, `/reload`
- **Ollama integration** — Local models with automatic tool-use fallback
- **Voice I/O** — Whisper STT + XTTS v2 TTS with voice cloning
- **RAG indexing** — tree-sitter AST parsing, 8 languages, SQLite FTS5 search
- **Smart routing** — Auto-route simple tasks to cheaper models
- **Cost dashboard** — Real-time token/cost tracking with budget limits
- **SDK mode** — `--headless` NDJSON server for editor/CI embedding ([docs](sdk/))
- **Session management** — Save, resume, search, export conversations
- **Sandboxing** — bwrap / firejail / strict isolation
- **XDG compliant** — Respects `$XDG_CONFIG_HOME`, `$XDG_DATA_HOME`, `$XDG_CACHE_HOME`

See **[FEATURES.md](FEATURES.md)** for the full reference.

---

## Documentation

| Document | Description |
|----------|-------------|
| [FEATURES.md](FEATURES.md) | Complete feature reference — every command, shortcut, and config option |
| [sdk/](sdk/) | SDK / headless mode — protocol, examples, integration guide |
| [CHANGELOG.md](CHANGELOG.md) | Release history |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute |
| [SECURITY.md](SECURITY.md) | Security policy and vulnerability reporting |

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
