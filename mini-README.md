# RustyClaw — quick reference

Rust port of rustyclaw. Native binary, no Node/Bun required.

## Run

```bash
cd ~/Claude-Source/RustyClaw
./target/release/rustyclaw

# Rebuild after changes
cargo build --release
```

## API keys

Add to `~/.env` — loaded automatically at startup:

```bash
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...        # for voice/Whisper API
OLLAMA_HOST=http://localhost:11434  # optional
```

No `source ~/.env` needed.

## Switch models mid-session

```
/model ollama:dolphin3      # any local Ollama model
/model ollama:qwen3:14b
/model default              # back to claude-sonnet-4-6
/model sonnet               # alias
```

## Voice input

```bash
# Setup (one-time)
sudo apt install ffmpeg          # or: brew install ffmpeg
pip install openai-whisper       # for offline transcription
echo 'OPENAI_API_KEY=sk-...' >> ~/.env  # for online transcription
```

```
/voice enable    ← enable voice
Ctrl+R           ← start recording
Ctrl+R           ← stop + transcribe into input box
```

## Key commands

| Command | Action |
|---------|--------|
| `/model` | Show or switch model |
| `/model ollama:<name>` | Switch to local Ollama model |
| `/voice enable\|disable` | Toggle voice input |
| `/sandbox [mode]` | Bash sandboxing (strict/bwrap/firejail/off) |
| `/session list` | List saved sessions |
| `/session search <q>` | Search sessions |
| `/tag add <tag>` | Tag current session |
| `/rename <name>` | Rename current session |
| `/share` | Export chat to markdown |
| `/share clip` | Copy chat to clipboard |
| `/teleport` | Export/import session as JSON |
| `/thinkback` | Token usage bar chart |
| `/notifications` | Toggle desktop notifications |
| `/edit-claude-md` | Edit CLAUDE.md in $EDITOR |
| `/compact` | Summarise conversation |
| `/clear` | Clear conversation + screen |
| `/banner [text\|none]` | Set org label in banner |
| `/plugin install <pkg>` | Install npm MCP plugin |
| `/mcp add <name> <cmd>` | Add MCP server |
| `/mcp remove <name>` | Remove MCP server |
| `/mcp disable\|enable <n>` | Disable/enable MCP server |
| `/upgrade` | Check for updates (GitHub) |
| `/copy` | Copy last reply to clipboard |
| `/ultraplan` | Deep planning prompt |
| `/autofix-pr [url]` | Fix PR review comments |
| `/commit` | Git commit via Claude |
| `/diff` | Git diff |
| `/doctor` | Diagnose environment |
| `/help` | All commands |
| `?` | Keyboard shortcuts overlay |

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send |
| `Shift+Enter` | Newline |
| `Esc` | Cancel request / close overlay |
| `Ctrl+R` | Voice record start/stop |
| `Tab` | Autocomplete command / history suggestion |
| `PgUp/PgDn` | Scroll chat |
| `Home/End` | Top / bottom |
| `Ctrl+W` | Delete word |
| `Ctrl+K` | Delete to end of line |
| `Ctrl+U` | Clear line |
| `Up/Down` | Input history |
| `?` | Shortcuts overlay |
