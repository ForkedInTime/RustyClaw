# RustyClaw Phase 3: Browser Automation, Watch Mode, Diff Review, Enhanced Skills

**Date:** 2026-04-12
**Status:** Approved
**Goal:** Close the four remaining competitive gaps to reach feature parity and beyond against all Rust Claude Code forks, Aider, Goose, and Cline.

---

## 1. Competitive Context

### Three-Way Source Analysis

Features cherry-picked from the best of three browser automation projects:

| Idea | Source | Why |
|---|---|---|
| CDP WebSocket client (tokio-tungstenite, no Node.js) | agent-browser (Vercel) | Rust-native, fastest, single-binary story |
| Accessibility tree + integer refs (@e1, @e2) | agent-browser + browser-use | LLM-friendly, no fragile CSS selectors |
| Annotated screenshots (numbered labels on elements) | agent-browser | Huge for vision models |
| Batch execution (multiple actions per call) | agent-browser | 93% more token-efficient than Playwright MCP |
| Snapshot diffing (before/after comparison) | agent-browser | Detect what changed |
| Autonomous agent loop with planning | browser-use (87K stars) | plan_update tool, exploration nudges, replan-on-stall |
| Loop detection + escalating nudges | browser-use | Action fingerprinting + page state hash, ~200 lines Rust |
| Structured extraction schemas | browser-use | JSON schema-driven: "give me {name, price, url}" |
| Message compaction for browser sessions | browser-use | Token-heavy context, auto-summarize older steps |
| Multi-LLM routing for browser subtasks | browser-use | Cheap model for extraction, expensive for planning |
| Custom action registry | browser-use | User-defined actions with typed params, maps to our skills |
| Sensitive data domain-locking | browser-use | Credentials tied to allowed_domains |
| Dialog handling (alerts, confirms) | Playwright MCP | agent-browser lacks this |
| Console message capture | Playwright MCP | Debug JS errors during automation |

### What We Skip

| Feature | Reason |
|---|---|
| Playwright code generation | We're an AI agent, not a test framework |
| CAPTCHA solver integration | Legal grey area, out of scope |
| iOS/Appium testing | Niche, massive complexity |
| WebSocket live streaming dashboard | Not MVP |
| Browser extension bridge | Breaks single-binary story |

---

## 2. Browser Automation (`src/browser/`)

### 2.1 Architecture

```
src/browser/
  mod.rs              # BrowserSession: launch, connect, close, lifecycle
  cdp.rs              # CDP WebSocket client (tokio-tungstenite), command/event dispatch
  snapshot.rs         # Accessibility tree extraction, integer refs, annotated screenshots
  actions.rs          # navigate, click, fill, screenshot, dialog, console, scroll, drag, upload, wait
  element.rs          # ClickableElementDetector: ARIA roles, JS listeners, label heuristics, scoring
  extraction.rs       # Schema-driven structured data extraction (JSON schema in, structured data out)
  planner.rs          # Agent loop: plan -> act -> evaluate -> replan (browser-use pattern)
  loop_detector.rs    # Action fingerprinting + page state SHA-256 hash + escalating nudge messages
  network.rs          # Request interception, HAR recording, route/mock/abort
  security.rs         # Sensitive data domain-locking, credential scoping per allowed_domains

src/tools/browser.rs  # 10+ BrowserTool variants implementing the Tool trait
```

### 2.2 Tool Surface for the AI

| Tool | Params | Returns |
|---|---|---|
| `browser_navigate` | `url` | page title, status code |
| `browser_snapshot` | optional `selector`, `depth` | accessibility tree with @refs |
| `browser_click` | `ref` (@e1) | success/failure, updated snapshot |
| `browser_fill` | `ref`, `value` | success |
| `browser_screenshot` | optional `full_page`, `annotated` | base64 PNG or saved file path |
| `browser_get_text` | `ref` or `selector` | extracted text |
| `browser_extract` | `url`, `schema` (JSON) | structured data matching schema |
| `browser_press_key` | `key` | success |
| `browser_wait` | `condition` (selector/text/timeout) | success/timeout |
| `browser_dialog` | `action` (accept/dismiss), `text` | success |
| `browser_console` | — | array of console messages |
| `browser_network_intercept` | `url_pattern`, `action` (mock/abort/log) | success |
| `browser_batch` | `actions[]` (array of above) | array of results |

### 2.3 Browser Session Lifecycle

- **Lazy start:** Session created on first browser tool call or `/browser` command.
- **Chrome discovery order:**
  1. Existing CDP endpoint via `CDP_ENDPOINT` env var
  2. Launch system Chrome: `--headless=new --remote-debugging-port=0`
  3. Chromium/google-chrome/chrome on PATH
- **Persistent within conversation:** Session survives across turns. Tabs, cookies, state maintained.
- **Cleanup:** Killed on `/clear`, session end, `/browser close`, or `App::clear()`.

### 2.4 CDP Client (`cdp.rs`)

- `tokio-tungstenite` WebSocket connection to Chrome DevTools Protocol.
- `AtomicU64` request ID sequencing for concurrent commands.
- Dual broadcast channels: typed CDP events + raw JSON for debug.
- 30-second keepalive pings to prevent proxy/firewall timeouts.
- Reconnect logic: if WebSocket drops, attempt re-attach to same browser instance.

### 2.5 Accessibility Snapshot (`snapshot.rs`)

- Uses CDP `Accessibility.getFullAXTree` command.
- Filters by interactive + content + structural ARIA roles.
- Each element gets an integer ref: `@e1`, `@e2`, `@e3`...
- Ref map stored in `BrowserSession` for resolution on subsequent actions.
- Compact mode: only interactive elements (forms, links, buttons).
- Annotated screenshot mode: overlays numbered labels on the rendered page matching refs.

### 2.6 Element Detection (`element.rs`)

Cherry-picked from browser-use's `ClickableElementDetector`:

- Score elements by: ARIA role weight, JS click event listeners, `cursor: pointer` CSS, label/aria-label presence, `tabindex` attribute, ancestor link/button wrapping.
- Higher-scored elements get lower ref numbers (more likely to be relevant).
- Invisible elements (display:none, visibility:hidden, zero-size) excluded.
- Deduplication: elements with identical text + role collapsed.

### 2.7 Planning Loop (`planner.rs`)

Adapted from browser-use's agent service:

```
loop {
    1. Get current state (snapshot + optional screenshot)
    2. Build prompt: system + plan + history + current state
    3. Send to LLM -> get AgentOutput { plan_update, actions[], evaluation }
    4. If evaluation.is_failure -> increment fail_count
    5. If fail_count >= 3 -> inject replan nudge
    6. Check loop_detector -> if stagnant, inject escalating nudge
    7. Execute actions (batch)
    8. Compact history if token count > threshold
    9. If goal_achieved or max_steps reached -> break
}
```

The planning loop only activates for autonomous browser tasks (e.g., `/browse "find the cheapest flight to Tokyo"`). Simple tool calls (AI calls `browser_click`) bypass the loop entirely.

### 2.8 Loop Detection (`loop_detector.rs`)

- Each action is fingerprinted: `hash(action_type + target_ref + value)`.
- Page state fingerprinted: `SHA-256(page_text_content)`.
- Rolling window of last 10 action+state pairs.
- If 3+ consecutive duplicates detected:
  - Level 1: "You seem to be repeating the same action. Try a different approach."
  - Level 2: "This action has failed multiple times. Consider: [alternative strategies]."
  - Level 3: "Stopping. The page may require authentication or the element may be dynamic."

### 2.9 Structured Extraction (`extraction.rs`)

```rust
pub struct ExtractionRequest {
    pub url: String,
    pub schema: serde_json::Value,  // JSON Schema defining desired output
    pub instructions: Option<String>,
}
```

- Navigate to URL, take snapshot, send to LLM with schema constraint.
- LLM returns JSON conforming to schema.
- Validation: parsed against schema, retry once on validation failure.
- Uses smart model router: extraction routed to cheapest capable model.

### 2.10 Security (`security.rs`)

- `SensitiveData` struct: `{ key, value, allowed_domains[] }`.
- Before `browser_fill` injects sensitive data, checks current page domain against allowed list.
- Credentials never logged in chat entries or session files.
- Domain validation: exact match or wildcard subdomain (`*.example.com`).

### 2.11 Slash Commands

| Command | Action |
|---|---|
| `/browser` or `/browse <url>` | Launch browser, navigate to URL |
| `/browser close` | Kill browser session |
| `/screenshot` | Take screenshot of current page |
| `/extract <url> <schema>` | Structured extraction |
| `/browse "natural language task"` | Autonomous browser agent with planning loop |

### 2.12 Voice Integration

No special wiring. The existing voice pipeline works naturally:

```
User speaks -> Whisper STT -> "go to github.com and find the trending repos"
  -> AI resolves to: browser_navigate + browser_snapshot + browser_get_text
  -> Executes via CDP
  -> AI summarizes: "Here are today's trending repos: 1. ..."
  -> XTTS speaks response
```

For autonomous tasks, voice input triggers the planning loop:
```
"scrape all product prices from amazon.com/deals" -> planner.rs loop
```

---

## 3. Watch Mode (`src/watch.rs`)

### 3.1 Architecture

Single module using the `notify` crate (cross-platform: inotify on Linux, FSEvents on macOS, ReadDirectoryChanges on Windows).

```rust
pub struct FileWatcher {
    watcher: RecommendedWatcher,
    debounce: Duration,           // 500ms default
    patterns: Vec<GlobPattern>,   // file filters
    marker_patterns: Vec<Regex>,  // AI:, TODO:, FIXME:
    rate_limit: Duration,         // 10s between auto-triggers
    last_trigger: Instant,
}
```

### 3.2 How It Works

1. `/watch` or `/watch src/ --pattern "*.rs"` starts the watcher.
2. `notify` fires events on file create/modify/delete.
3. Debounce: 500ms after last change event before processing.
4. On trigger, scan changed files for action markers:
   - `// AI: <instruction>` -> sent as prompt to the AI immediately
   - `// TODO:` / `// FIXME:` -> collected, shown as suggestions (not auto-acted)
5. Rate limit: max 1 auto-trigger per 10 seconds to prevent feedback loops.
6. The AI sees: `"File src/auth.rs changed at line 42. Marker found: AI: add retry logic for database timeouts"`
7. Auto-fix integration: if a watched file change causes lint/test failure and `autoFixLoop` is enabled, the auto-fix loop kicks in automatically.

### 3.3 Slash Commands

| Command | Action |
|---|---|
| `/watch` | Watch cwd, react to AI: markers |
| `/watch src/` | Watch specific directory |
| `/watch --pattern "*.rs"` | Filter by glob |
| `/watch off` | Stop watching |
| `/watch status` | Show what's being watched + pending changes |

### 3.4 TUI Integration

- Status bar indicator when active: `watching src/ (2 pending)`
- Watch events appear as system entries in the chat.
- User can review pending TODO/FIXME markers via `/watch status`.

### 3.5 Safety

- Only `AI:` markers trigger automatic AI action. `TODO:`/`FIXME:` are passive.
- Rate limiting prevents infinite loops (AI edits file -> triggers watch -> AI edits again).
- Marker is consumed after processing: the AI removes the `// AI:` comment as part of its edit.
- Respects `.gitignore` — doesn't watch ignored files.
- Max queue depth: 20 pending changes. Oldest dropped if exceeded.

---

## 4. Diff Review UI (`src/tui/diff.rs`)

### 4.1 Architecture

New rendering module + new `Overlay` variant in the existing overlay system.

```rust
pub struct DiffOverlay {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub hunk_states: Vec<HunkState>,  // Accepted, Rejected, Pending
    pub selected_hunk: usize,
    pub scroll: usize,
}

pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: Vec<String>,
    pub new_start: usize,
    pub new_lines: Vec<String>,
    pub context_before: Vec<String>,
    pub context_after: Vec<String>,
}

pub enum HunkState { Pending, Accepted, Rejected }
```

### 4.2 Rendering

- Unified diff format in the overlay (color-coded):
  - Green (`+`) for additions
  - Red (`-`) for deletions
  - Dimmed for context lines
  - Yellow highlight on the currently selected hunk
- File header with path and change summary (`+15 -8`)
- Multi-file support: navigate between files with `[` and `]`

### 4.3 Keyboard Controls

| Key | Action |
|---|---|
| `j`/`k` or `Up`/`Down` | Navigate between hunks |
| `Space` | Toggle current hunk accept/reject |
| `y` | Accept all hunks |
| `n` | Reject all hunks |
| `[` / `]` | Previous/next file |
| `e` | Open file in `$EDITOR` at hunk line |
| `Enter` | Apply decisions (accepted hunks kept, rejected hunks reverted) |
| `q` / `Esc` | Dismiss without applying |

### 4.4 Diff Source

- **`/diff`**: Runs `git diff` on uncommitted changes. Parses unified diff output.
- **`/diff --last`**: Uses autocommit shadow refs to diff only the last AI turn's changes.
- **`/diff <file>`**: Diff for a specific file only.
- **Auto-trigger**: If `diffReview: true` in settings.json, the diff overlay opens automatically after each AI turn that modifies files, before the AI continues.

### 4.5 Integration with Autocommit

When the user accepts/rejects hunks:
1. Accepted hunks: already on disk (no action needed).
2. Rejected hunks: reverted using `git checkout -p` equivalent (patch-level restore from shadow ref).
3. A new shadow ref snapshot is taken after the user's decisions.

### 4.6 Slash Commands

| Command | Action |
|---|---|
| `/diff` | Show all uncommitted changes |
| `/diff src/main.rs` | Show changes for specific file |
| `/diff --last` | Changes from last AI turn only |
| `/diff --staged` | Show staged changes |

### 4.7 Settings

```jsonc
// settings.json
{
    "diffReview": false,       // auto-show diff after AI edits (default off)
    "diffStyle": "unified"     // "unified" or "side-by-side" (future)
}
```

---

## 5. Enhanced Skills / Recipes (`src/skills/mod.rs` extensions)

### 5.1 Enhanced Skill Format

Backward-compatible extension of the existing skill markdown format.

**Before (still works):**
```markdown
# Skill Name
Description.
---
Prompt with {{ARGS}}.
```

**After (new capabilities):**
```markdown
---
name: scrape-prices
description: Extract product prices from an e-commerce page.
category: browser
params:
  url: { required: true, description: "Target URL" }
  max_pages: { default: 3, description: "Max pagination depth" }
  output: { default: "prices.md", description: "Output file" }
  format: { default: "markdown", enum: ["markdown", "csv", "json"] }
---
Navigate to {{url}} and extract product prices.

1. Take an accessibility snapshot of the page.
2. Find all elements with role "listitem" or class matching product/item.
3. For each product, extract: name, price, availability.
4. Output as a {{format}} table to {{output}}.
5. If there's a "Next page" link, follow it (max {{max_pages}} pages).
```

### 5.2 Changes to `src/skills/mod.rs`

```rust
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt_template: String,
    // New fields:
    pub category: Option<String>,           // "browser", "code", "workflow", "analysis"
    pub params: Vec<SkillParam>,            // named parameters with defaults/validation
}

pub struct SkillParam {
    pub name: String,
    pub required: bool,
    pub default: Option<String>,
    pub description: String,
    pub enum_values: Option<Vec<String>>,   // constrained choices
}
```

### 5.3 Parameter Resolution

Invocation: `/scrape-prices url=https://example.com max_pages=5`

Resolution order:
1. Parse `key=value` pairs from args string.
2. For missing params: use `default` if defined, error if `required`.
3. For `enum` params: validate value is in allowed set.
4. Replace `{{param_name}}` placeholders in template.
5. Fallback: if no `key=value` pairs found, treat entire args as `{{ARGS}}` (backward compat).

### 5.4 Category Filtering

- `/skills` — list all skills (unchanged behavior).
- `/skills browser` — list only browser-category skills.
- `/skills code` — list only code-category skills.

### 5.5 Browser Context Injection

When a skill with `category: browser` is invoked:
1. Browser session is auto-started if not already running.
2. System prompt appended: "You have an active browser session. Use browser_* tools to complete this task."
3. After skill completes, browser session stays open (user can continue interacting).

### 5.6 Example Skills Library

Ship a set of example browser skills in `~/.claude/skills/examples/` (not auto-loaded, but discoverable via `/skills examples`):

| Skill | Purpose |
|---|---|
| `scrape-prices.md` | Extract product prices with pagination |
| `login-and-scrape.md` | Authenticate then extract data |
| `screenshot-compare.md` | Before/after visual diff of a URL |
| `form-test.md` | Fill and submit a form, verify success |
| `monitor-status.md` | Check if a site/API is up |
| `extract-docs.md` | Scrape API documentation into markdown |
| `check-lighthouse.md` | Run Lighthouse audit via CDP |

---

## 6. Dependencies

### New Crate Dependencies

| Crate | Purpose | Size Impact |
|---|---|---|
| `tokio-tungstenite` | CDP WebSocket | Already in dependency tree via existing deps |
| `notify` | Filesystem watcher | ~50KB, pure Rust |
| `image` | Screenshot processing | ~200KB (may already be pulled transitively) |
| `sha2` | Page state fingerprinting for loop detection | ~30KB |

### Shared with Existing

`tokio`, `reqwest`, `serde`, `serde_json`, `futures-util` — already in `Cargo.toml`.

Estimated binary size increase: **~300KB** (from ~5MB to ~5.3MB).

---

## 7. Settings

New fields in `settings.json`:

```jsonc
{
    // Browser
    "browserEnabled": true,
    "browserHeadless": true,
    "browserChromePath": null,        // auto-detect if null
    "browserCdpEndpoint": null,       // connect to existing if set
    "browserDefaultTimeout": 30000,   // ms
    
    // Watch mode
    "watchEnabled": false,            // must opt-in
    "watchDebounce": 500,             // ms
    "watchRateLimit": 10000,          // ms between auto-triggers
    "watchMarkers": ["AI:", "AGENT:"],
    
    // Diff review
    "diffReview": false,              // auto-show after AI edits
    "diffStyle": "unified",
    
    // Extraction
    "extractionModel": null           // override model for extraction (cheap model)
}
```

---

## 8. Implementation Order

Recommended sequence based on dependencies and competitive impact:

1. **Browser core** (`cdp.rs`, `browser/mod.rs`, `snapshot.rs`, `actions.rs`, `element.rs`) — the foundation everything else builds on.
2. **Browser tools** (`tools/browser.rs`) + slash commands — makes browser usable by AI and user.
3. **Enhanced skills** (`skills/mod.rs` extensions) — named params, categories, browser context injection.
4. **Diff review** (`tui/diff.rs`) — independent of browser, high user demand.
5. **Planning loop + loop detection** (`planner.rs`, `loop_detector.rs`) — autonomous browser agent.
6. **Extraction** (`extraction.rs`) — builds on snapshot + smart model router.
7. **Watch mode** (`watch.rs`) — independent of browser, uses `notify` crate.
8. **Network interception** (`network.rs`) + security (`security.rs`) — advanced features, ship last.

---

## 9. The Pitch After This Ships

"A single Rust binary that indexes your codebase, routes tasks to the cheapest model, runs parallel agents in worktrees, controls a real browser via CDP with reusable automation skills, speaks in your voice, watches your files and auto-reacts to TODO markers, reviews every diff inline before committing, shows you every token spent, and works offline via Ollama. Sub-50ms startup. Zero dependencies. Zero flickering."

No tool in the world offers this combination.
