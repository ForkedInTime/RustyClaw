# RustyClaw

A Rust port of [Clawd-Code](../rustyclaw/) — a personal fork of [Claude Code](https://github.com/anthropics/claude-code), Anthropic's official CLI for Claude.

Provides the same capabilities as rustyclaw (Anthropic Claude + native Ollama) in a single native binary with no Node.js/Bun runtime dependency.

---

## Status

**Feature-complete.** All core features implemented, performance-optimized, and in daily use.

---

## Features

### AI Backends
- **Anthropic Claude** — full streaming, tool use, all Claude models
- **Ollama (local models)** — native integration, no proxy required
  - Switch mid-session: `/model ollama:dolphin3`, `/model ollama:llama3.2`, etc.
  - `/model default` → back to `claude-sonnet-4-6`
  - Ollama models get their own system prompt (direct, uncensored assistant)
  - Auto-detects when a model doesn't support tools and retries without them
  - Tab-completion for model names (queries `ollama list` live)

### Tools
| Tool | Description |
|------|-------------|
| `Bash` | Shell command execution with live streaming output |
| `Read` | File reading |
| `Write` | File creation/overwriting |
| `Edit` | Targeted in-place edits |
| `MultiEdit` | Multiple edits in one call |
| `Glob` | File pattern matching |
| `Grep` | Regex content search |
| `WebFetch` | Fetch and parse web pages |
| `WebSearch` | Web search (Anthropic key) |
| `Agent` | Spawn sub-agents for parallel tasks |
| `TodoWrite` | Task list management |
| `TaskCreate/Get/List/Update/Stop/Output` | Background task management |
| `EnterWorktree/ExitWorktree` | Git worktree isolation |
| `AskUserQuestion` | Request user input mid-task |
| `EnterPlanMode/ExitPlanMode` | Toggle planning-only mode |
| `MemoryRead/MemoryWrite` | Persistent markdown memory |
| `NotebookRead/NotebookEdit` | Jupyter notebook support |
| `LSP` | Language server protocol integration |
| `WebBrowser` | Headless browser automation |
| `PowerShell` | Windows PowerShell (cross-platform) |
| `Sleep` | Pause between steps |
| `DiscoverSkills` | Find available skills |
| `SkillTool` | Execute skill files |
| `WorkflowTool` | Multi-step workflow execution |
| `CronCreate/Delete/List` | Schedule recurring tasks |
| `ConfigTool` | Read/write config values |
| `ToolSearch` | Search available tools by description |
| `MCP tools` | All tools from connected MCP servers (prefixed `mcp__<server>__`) |

### Terminal UI
- Built with [ratatui](https://github.com/ratatui-org/ratatui) — inline viewport (no alternate screen)
- Native text selection and terminal scrollback work normally
- Mouse wheel scroll (3 lines per tick); hold Shift to select text
- Welcome banner with pixel-art mascot, recent sessions, model/org info
- Collapsible bash output: last 4 lines shown + `[▸ N lines]` header
- Scroll position indicator badge when scrolled up
- `?` key → keyboard shortcuts overlay (Esc to dismiss)
- Vim mode (`:` / `i` to toggle)
- Tab autosuggestion — ghost text completions from input history
- `/clear` properly scrolls old content off screen (no stacked banners)

### Voice Input ✓
- `/voice` — enable/disable voice input
- `Ctrl+R` — start/stop recording (when voice is enabled)
- Audio capture via **ffmpeg** (preferred), **arecord**, or **sox** — auto-detected
- Transcription via **local Whisper** (offline) or **OpenAI Whisper API** (online)
- Transcribed text is inserted directly into the input box
- API key auto-loaded from `.env` files — no manual export needed

### Session Management
- Sessions auto-saved and resumable (`-r <id>` or `/resume <id>`)
- `/session list` — list all sessions
- `/session search <query>` — search sessions by name/preview
- `/session clear-all` — delete all sessions except current
- `/export` — export conversation to markdown file
- `/share` — export to markdown file; `/share clip` → copy to clipboard
- `/teleport` — export/import full session context as JSON (move between machines)
- `/compact` — summarise conversation to free context window

### Sandboxing ✓
- `/sandbox` — show current sandbox mode
- `/sandbox strict` — block all network + filesystem writes
- `/sandbox bwrap` — Linux bubblewrap isolation
- `/sandbox firejail` — firejail isolation
- `/sandbox off` — disable sandboxing
- `/sandbox network on|off` — toggle network access within sandbox

### Notifications ✓
- `/notifications` — enable/disable desktop + terminal bell notifications
- Desktop notification (via `notify-send`) fired when a long task completes
- Terminal bell as fallback

### Analytics & Cost Tracking ✓
- Per-turn token usage tracked in `app.turn_costs`
- `/thinkback` — ASCII bar chart of token usage across all turns in the session

### Slash Commands

**Core**
| Command | Description |
|---------|-------------|
| `/help` | Show all commands and tools |
| `/clear` | Clear conversation history and screen |
| `/compact` | Summarise conversation to free context |
| `/exit` | Exit |
| `/vim` | Toggle vim mode |
| `?` key | Keyboard shortcuts overlay |

**Model**
| Command | Description |
|---------|-------------|
| `/model [name]` | Show or switch model (persists to settings.json) |
| `/model ollama:<name>` | Switch to local Ollama model |
| `/model default` | Switch back to `claude-sonnet-4-6` |

**Session**
| Command | Description |
|---------|-------------|
| `/session list` | List saved sessions |
| `/session search <q>` | Search sessions by name or preview |
| `/session clear-all` | Delete all sessions |
| `/rename <name>` | Rename current session |
| `/resume <id>` | Resume a previous session |
| `/export` | Export chat to markdown |
| `/share` | Export chat to markdown file |
| `/share clip` | Copy chat to clipboard |
| `/teleport` | Export/import session context as JSON |
| `/tag [add\|remove\|list] <tag>` | Tag sessions for organisation |
| `/compact` | Summarise + compress context |

**MCP / Plugins**
| Command | Description |
|---------|-------------|
| `/mcp` | List connected MCP servers |
| `/mcp add <name> <cmd\|url>` | Add MCP server to `~/.claude/settings.json` |
| `/mcp remove <name>` | Remove MCP server |
| `/mcp enable <name>` | Re-enable a disabled server |
| `/mcp disable <name>` | Disable a server without removing it |
| `/mcp get <name>` | Show server config |
| `/mcp tools` | List tools per server |
| `/plugin install <package>` | Install npm plugin as MCP server |
| `/plugin list` | List installed plugins |
| `/plugin remove <name>` | Remove a plugin |
| `/reload-plugins` | Show plugin/MCP count (requires restart) |

**Voice & UI**
| Command | Description |
|---------|-------------|
| `/voice` | Show voice input status and setup |
| `/voice enable\|disable` | Enable or disable voice input |
| `/notifications` | Toggle desktop/bell notifications |
| `/banner [text\|none]` | Set or clear the org label in the banner |
| `/theme [name]` | Switch colour theme |
| `/thinkback` | ASCII bar chart of per-turn token usage |

**Tools & Config**
| Command | Description |
|---------|-------------|
| `/sandbox [mode]` | Show or set sandbox mode |
| `/sandbox network on\|off` | Toggle network in sandbox |
| `/hooks` | Show configured tool hooks |
| `/memory` | Show CLAUDE.md memory files |
| `/edit-claude-md` | Open CLAUDE.md in `$EDITOR` |
| `/permissions` | Show tool allow/deny rules |
| `/config` | Show active configuration |
| `/doctor` | Diagnose environment (keys, tools, plugins) |
| `/env` | Show environment variables |

**Productivity**
| Command | Description |
|---------|-------------|
| `/tasks` | Show current TodoWrite list |
| `/skills` | List available skill files |
| `/agents` | Show agent tools + swarm status |
| `/commit [msg]` | Commit changes via git |
| `/commit-push-pr [msg]` | Commit, push, and open PR |
| `/pr_comments [url]` | Fetch and address PR review comments |
| `/autofix-pr [url]` | Tell Claude to fix all PR comments |
| `/issue <title>` | Create a GitHub issue |
| `/diff [path]` | Show git diff |
| `/branch` | Show git branch info |
| `/summary` | Summarise the conversation |
| `/ultraplan [deep]` | Deep-analysis planning prompt |
| `/advisor [topic]` | Expert advisor prompt |
| `/effort [high\|medium\|low]` | Set response effort level |
| `/insights` | Summarise recent work + insights |
| `/btw <note>` | Leave a note for Claude mid-task |
| `/ctx-viz` | Visualise context window usage |
| `/stats` | Show session + API stats |
| `/cost` | Show token cost for this session |
| `/usage` | Show token usage breakdown |
| `/copy` | Copy last assistant reply to clipboard |

**Meta**
| Command | Description |
|---------|-------------|
| `/feedback` | Show feedback/bug report info |
| `/terminal-setup` | Show terminal compatibility tips |
| `/release-notes` | Show recent changes |
| `/upgrade` | Check for updates on GitHub |
| `/version` | Show binary version info |
| `/statusline [on\|off\|format]` | Manage shell status line |

### Configuration
- Reads `~/.claude/settings.json` and `./.claude/settings.json` (project wins)
- Loads `CLAUDE.md` from global (`~/.claude/`) and project hierarchy — auto-injected into system prompt
- `bannerOrgDisplay` in `~/.claude/config.json` — custom label shown in banner (set via `/banner`)
- API keys auto-loaded from `.env` files at startup (see API Keys section below)

---

## API Keys & `.env` Support

RustyClaw automatically loads `.env` files at startup — no need to `source ~/.env` manually.

**Search order** (first match wins, existing env vars always take priority):
1. `$CWD/.env` — project-local keys
2. `~/.env` — user-global keys
3. `~/.config/rustyclaw/.env` — app-specific config

**Recommended setup:**
```bash
# Add your keys to ~/.env (create it if it doesn't exist)
echo 'ANTHROPIC_API_KEY=sk-ant-...' >> ~/.env
echo 'OPENAI_API_KEY=sk-...' >> ~/.env      # for voice/Whisper API
echo 'OLLAMA_HOST=http://localhost:11434' >> ~/.env  # optional
```

Then just run `./target/release/rustyclaw` — no sourcing needed.

**Supported keys:**
| Key | Purpose |
|-----|---------|
| `ANTHROPIC_API_KEY` | Claude API (required for Claude models) |
| `OPENAI_API_KEY` | OpenAI Whisper API for voice transcription |
| `WHISPER_API_KEY` | Alternative key name for Whisper API |
| `OLLAMA_HOST` | Ollama server URL (default: `http://localhost:11434`) |

---

## Build & Run

```bash
cd ~/Claude-Source/RustyClaw

# Build release binary
cargo build --release

# Run — .env files loaded automatically
./target/release/rustyclaw

# Or with explicit key export (overrides .env)
ANTHROPIC_API_KEY=sk-ant-... ./target/release/rustyclaw

# Resume a previous session
./target/release/rustyclaw -r <session-id>
```

---

## Voice Input Setup

```bash
# Install audio capture (pick one)
sudo apt install ffmpeg          # preferred (Linux)
brew install ffmpeg              # macOS

# Install local Whisper for offline transcription (optional)
pip install openai-whisper

# Add your OpenAI API key for online transcription (optional)
echo 'OPENAI_API_KEY=sk-...' >> ~/.env
```

Then inside RustyClaw:
```
/voice enable
Ctrl+R   ← start recording
Ctrl+R   ← stop and transcribe
```

---

## Ollama Integration

Any model installed in Ollama works out of the box.

```bash
# Install a model
ollama pull dolphin3

# Start RustyClaw
./target/release/rustyclaw

# Switch to Ollama model inside the session
/model ollama:dolphin3
/model ollama:llama3.2:latest

# Switch back to Claude
/model default
```

**How it works:**
- Requests to `ollama:*` models are routed to Ollama's OpenAI-compatible `/v1/chat/completions` endpoint
- Tool calls (Bash, Read, Write, etc.) are fully translated in both directions
- Streaming SSE is translated on-the-fly — no buffering
- Conversation history is model-agnostic: switching mid-session carries the full transcript
- Models that don't support tools get a text-only retry automatically

---

## Architecture

```
src/
├── main.rs              # Entry point, CLI args (clap), .env auto-load (3-level cascade)
├── api/
│   ├── mod.rs           # ApiBackend enum — routes to Anthropic or Ollama
│   ├── anthropic.rs     # Anthropic Messages API client (streaming SSE)
│   ├── ollama.rs        # Ollama OpenAI-compat client + tool translation
│   └── types.rs         # Shared message/content types
├── tui/
│   ├── run.rs           # Event loop: tokio::select! + EventStream; /clear screen clear
│   ├── render.rs        # ratatui layout — banner, chat, input, status; ThemeColors
│   ├── app.rs           # App state, cached_cwd, model_short, pending_screen_clear, turn_costs
│   ├── events.rs        # AppEvent enum (30+ variants)
│   └── markdown.rs      # Markdown → ratatui spans renderer (zero-alloc tokenizer)
├── tools/               # 30+ tool implementations
│   ├── bash.rs          # Streaming bash with live output + sandbox integration
│   ├── file_read.rs, file_write.rs, file_edit.rs, multi_edit.rs
│   ├── glob.rs, grep.rs
│   ├── web_fetch.rs, web_search.rs, web_browser.rs
│   ├── agent.rs         # Sub-agent spawning
│   ├── todo.rs          # Shared TodoWrite state
│   ├── tasks.rs         # Background task registry (TaskCreate/Get/List/Update/Stop/Output)
│   ├── worktree.rs      # Git worktree isolation (EnterWorktree/ExitWorktree)
│   ├── ask_user.rs      # AskUserQuestion — mid-task user input
│   ├── plan_mode.rs     # EnterPlanMode/ExitPlanMode
│   ├── memory.rs        # MemoryRead/MemoryWrite (markdown persistence)
│   ├── notebook.rs      # Jupyter notebook read/edit
│   ├── lsp.rs           # Language server protocol
│   ├── cron.rs          # CronCreate/Delete/List
│   ├── config_tool.rs   # ConfigTool
│   ├── tool_search.rs   # ToolSearch
│   ├── skill_tool.rs    # SkillTool
│   ├── workflow.rs      # WorkflowTool
│   ├── sleep.rs, powershell.rs
│   ├── send_message.rs, team_tools.rs  # Agent swarm (experimental)
│   ├── mcp_resources.rs # MCP resource listing/reading
│   └── mod.rs           # Tool trait, registry, ToolContext, snapshot_file
├── commands/
│   └── mod.rs           # 60+ slash commands + CommandAction dispatch
│                        # Includes /mcp add|remove|enable|disable, /plugin, /tag, /upgrade, /ultraplan
├── mcp/                 # MCP client infrastructure
│   ├── client.rs        # MCP server connection (stdio + HTTP)
│   ├── types.rs         # Protocol types
│   └── mod.rs           # Server startup, tool wrapping
├── voice.rs             # Recording backends (ffmpeg/arecord/sox), Whisper transcription
├── sandbox.rs           # strict/bwrap/firejail sandboxing
├── config.rs            # Config loading, CLAUDE.md injection, banner label, settings persistence
├── settings.rs          # settings.json parsing (model, voice, sandbox, notifications, hooks)
├── session/
│   └── mod.rs           # Session save/resume/list/delete/export/search/tag
├── skills.rs            # Skills file discovery and loading
├── permissions/         # Tool permission checking + allow/deny rules
├── compact.rs           # Context compaction (summarise + compress history)
├── hooks.rs             # Pre/post tool hooks (notify-send, bell, custom commands)
└── plugins.rs           # Plugin registry (~/.claude/plugins.json)
```

### Key Crates

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` |
| Terminal UI | `ratatui` (with `unstable-rendered-line-info`) |
| Terminal events | `crossterm` (with `event-stream`) |
| HTTP / API | `reqwest` |
| SSE streaming | `eventsource-stream` + `futures-util` |
| JSON | `serde_json` |
| CLI args | `clap` |
| Error handling | `anyhow` / `thiserror` |
| Home dir | `dirs` |
| UUID | `uuid` |

---

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | Insert newline |
| `Esc` | Cancel in-flight request / close overlay |
| `?` | Show keyboard shortcuts overlay |
| `Ctrl+R` | Start/stop voice recording (when voice enabled) |
| `PgUp` / `PgDn` | Scroll chat |
| `Home` / `End` | Scroll to top / bottom |
| `Ctrl+A` / `Ctrl+E` | Cursor to start/end of line |
| `Ctrl+W` | Delete word back |
| `Ctrl+K` | Delete to end of line |
| `Ctrl+U` | Clear line |
| `Alt+B` / `Alt+F` | Word back/forward |
| `Tab` | Autocomplete slash commands, Ollama model names, or history suggestion |
| `Up` / `Down` | Navigate input history |

---

## Performance

Release build is optimized for low latency and minimal resource use:

| Metric | Value |
|--------|-------|
| Startup time | ~10ms |
| Memory usage | ~10MB |
| Binary size | ~8MB (stripped) |
| CPU at idle | ~0% |

**Key optimizations:**
- `ThemeColors` computed once per frame (not per sub-function)
- `cached_cwd` and `model_short` in App — no per-frame syscalls or string allocs
- Overlay markdown pre-rendered once at creation, not every frame
- Terminal size cached — `ioctl` only on resize events
- Zero-alloc tokenizer in `markdown.rs`
- `lto = "fat"`, `codegen-units = 1`, `panic = "abort"` in release profile

---

## Compared to TS rustyclaw

| Feature | TS rustyclaw | RustyClaw |
|---------|--------------|----------------|
| Runtime | Bun + Node.js | Native binary (no runtime) |
| UI framework | React + Ink | ratatui |
| Startup | ~300ms | ~10ms |
| Memory | ~150MB | ~10MB |
| Binary size | ~50MB (JS bundle) | ~8MB |
| Ollama support | Yes | Yes |
| Tool support | Full | Full (30+ tools) |
| Text selection | Yes (inline mode) | Yes (Viewport::Inline) |
| Sessions | Yes | Yes |
| Session tags | Yes | Yes |
| Skills | Yes | Yes |
| Voice input | Yes | Yes (ffmpeg/arecord/sox + Whisper) |
| Sandbox | No | Yes (strict/bwrap/firejail) |
| `.env` auto-load | No | Yes (3-level cascade) |
| Desktop notifications | No | Yes (notify-send) |
| Token usage chart | No | Yes (/thinkback) |
| MCP add/remove via CLI | Yes (claude mcp add) | Yes (/mcp add\|remove\|enable\|disable) |
| Plugin system | Yes (/plugin install) | Yes (/plugin install\|list\|remove) |
| Upgrade check | Yes | Yes (/upgrade → GitHub releases API) |
| Copy to clipboard | Yes | Yes (wl-copy/xclip/xsel/pbcopy/clip.exe) |
| Plan mode | Yes | Yes (EnterPlanMode/ExitPlanMode tools) |
| Worktrees | Yes | Yes (EnterWorktree/ExitWorktree tools) |
| Jupyter notebooks | Yes | Yes (NotebookRead/NotebookEdit) |
| Background tasks | Yes | Yes (TaskCreate/Get/List/Update/Stop) |
| Cron scheduling | Yes | Yes (CronCreate/Delete/List) |

---

## License

Derived from Claude Code and rustyclaw. For personal/educational use.
