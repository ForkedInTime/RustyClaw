# Auto Lint/Test Loop — Design Spec

**Status:** Approved
**Date:** 2026-04-10
**Phase:** 2 (CLAUDE.md "NEXT UP" item #1)
**Author:** RustyClaw principal engineer

---

## Problem

When the model edits code, there is nothing stopping it from shipping broken
output. The existing `auto_rollback` module runs tests once per turn and, on
failure, reverts the touched files via `git checkout --`. This protects the
working tree but wastes the work the model already did — the user sees a
vague error, the edits disappear, and the model never learns it was wrong.

[redacted] and [redacted] solved this years ago with a tight loop: lint + test after
every edit, feed failures back to the model as a new user turn, iterate until
green or a retry cap is hit. This is their single most-cited killer feature
and we have no answer to it.

Phase 2 needs this feature, and the existing `auto_rollback` scaffold is the
right starting point.

---

## Goals

1. After the model's Write/Edit/MultiEdit tool calls complete, automatically
   run lint + tests and feed any failures back to the model as a new turn,
   iterating up to `maxRetries` times.
2. Support the four primary ecosystems (Rust, Node, Python, Go) out of the
   box with zero configuration.
3. Keep the user in control — nothing runs unless autonomy mode allows it,
   and cancellation (Esc), budget caps, and turn limits all still apply.
4. Preserve the in-progress work on the filesystem even when the loop gives
   up. The user inspects and decides; RustyClaw does not silently revert.

## Non-goals

- Editor/linter auto-fix (`clippy --fix`, `eslint --fix`, etc.) — the model
  does the fixing. Auto-fix tools would race the model and muddy blame.
- Per-file lint invocation. Lint runs against the whole project because that
  matches how every supported linter is typically run in CI.
- Concurrent lint + test execution. Sequential with fast-fail on lint is
  cheaper, simpler, and matches user intuition ("lint is fast, tests are
  slow").
- A new slash command (`/autofix on|off`). Configure via `settings.json`
  for now; a command can follow if users ask.
- Retry across separate user prompts. The loop is scoped to one user turn.

---

## Decisions (locked before implementation)

### D1. Replace revert with retry

The existing `git checkout -- <files>` path on test failure is **removed**.
On failure, RustyClaw injects a synthetic user message containing the lint
and test output and re-calls the model, up to `maxRetries` times. On cap
reached, the loop stops, the working tree is left as-is, a SystemMessage
summarises the failures, and the turn ends normally with a `Done` event so
partial work is preserved in history.

Rationale: adding retry *and* keeping revert reintroduces the same "wait,
why did my code disappear?" surprise that makes [redacted] users love [redacted] — it
never silently reverts. Users who want atomic safety use git and the
existing `/undo` command.

### D2. Rename `rollback.rs` → `autofix.rs`

Once the revert path is gone, `rollback.rs` is a misnomer: the module
contains detection, trigger rules, and a test runner, but it no longer
rolls anything back. Rename:

- File: `src/rollback.rs` → `src/autofix.rs` (via `git mv`)
- Module: `crate::rollback` → `crate::autofix`
- Struct: `RollbackConfig` → `AutoFixConfig`
- Enum: `RollbackTrigger` → `AutoFixTrigger`
- Settings: `AutoRollbackSettings` → `AutoFixSettings`, field `auto_rollback`
  → `auto_fix`
- JSON key: `autoRollback` remains as a silent `#[serde(alias = ...)]` so
  existing user configs keep working with zero migration

`git_restore_files` is deleted entirely (dead code after D1).

### D3. Feedback is a synthetic `Role::User` text message

The Anthropic API requires every `ToolResult` block to reference a matching
`ToolUse` ID from the immediately-preceding assistant turn. Injecting a
fake ToolResult would either crash the API call or corrupt history. The
only valid shape is `Message { role: User, content: vec![Text { ... }] }`
appended to the conversation. This matches [redacted]/[redacted] and reads naturally
to the model as "the user is telling me my last edit broke something."

### D4. Feedback message format (anti-cheat included)

```text
Your last edits failed automated checks. Fix the issues below.

## Lint (<actual lint command>)
<first ~2KB of trimmed lint stderr, or "(no output)">

## Tests (<actual test command>)
<first ~2KB of trimmed test stderr, or "(skipped: lint failed)">

Make the minimum edits required to make both pass. Do not disable lints,
skip tests, or add `#[allow(...)]` / `# type: ignore` / `eslint-disable` /
`//nolint` unless the original code had them. If a test assertion is
genuinely wrong, explain why before changing it.
```

The anti-cheat paragraph matters: without it, clippy-under-retry converges
on `#[allow(dead_code)]` and the equivalent escape hatches in other
ecosystems. [redacted] learned this the hard way. The "explain why before
changing" clause gives the model a legitimate escape valve for actual bugs
in tests.

Truncation is hard-capped at 2048 bytes per section to keep context cost
predictable across retries.

---

## Architecture

### Turn flow (after the change)

```
User prompt
    ↓
Agentic loop turn N
    ├─ API call → assistant response
    ├─ Execute tool uses (Write/Edit/MultiEdit populate touched_files)
    ├─ All tools done, any files touched & should_trigger() true?
    │   ├─ No  → continue loop normally
    │   └─ Yes → run_checks(lint_cmd, test_cmd)
    │            ├─ Pass → continue loop normally
    │            └─ Fail
    │                 ├─ retries_used < max_retries?
    │                 │   ├─ Yes → inject feedback as User Text msg
    │                 │   │        retries_used += 1
    │                 │   │        emit SystemMessage("[auto-fix] retry N/M")
    │                 │   │        clear touched_files
    │                 │   │        loop back to API call
    │                 │   └─ No  → emit SystemMessage with failure summary
    │                 │            emit Done (preserves partial work)
    │                 │            return
```

### Modules

**`src/autofix.rs`** (renamed from `rollback.rs`, expanded):

```rust
pub struct AutoFixConfig {
    pub enabled: bool,
    pub trigger: AutoFixTrigger,
    pub lint_command: Option<String>,   // NEW
    pub test_command: Option<String>,
    pub max_retries: u32,               // was "reserved"; now active
    pub timeout_secs: u64,
}

pub enum AutoFixTrigger { Autonomous, Always, Off }

pub enum CheckOutcome {
    Pass,
    Fail { lint_stderr: Option<String>, test_stderr: Option<String> },
    Skipped { reason: String },
    NoRunners,
}

pub fn detect_test_command(cwd: &Path, override_cmd: &Option<String>) -> Option<String>;
pub fn detect_lint_command(cwd: &Path, override_cmd: &Option<String>) -> Option<String>;
pub fn should_trigger(config: &AutoFixConfig, autonomy_mode: &str) -> bool;
pub fn run_checks(
    cwd: &Path,
    lint_cmd: Option<&str>,
    test_cmd: Option<&str>,
    timeout_secs: u64,
) -> CheckOutcome;
pub fn format_feedback_message(
    lint_cmd: Option<&str>,
    test_cmd: Option<&str>,
    lint_stderr: Option<&str>,
    test_stderr: Option<&str>,
) -> String;

// Deleted: git_restore_files, git_stash_create (unused after D1)
// Kept: run_tests (called by run_checks), TestResult internally
```

**Detection tables** (new lint detection, test detection unchanged):

| Project file      | Lint command                                  | Test command           |
|-------------------|-----------------------------------------------|------------------------|
| `Cargo.toml`      | `cargo clippy --all-targets -- -D warnings`   | `cargo test`           |
| `package.json`    | `npx --no-install eslint .`                   | `npm test`             |
| `pyproject.toml`  | `ruff check .`                                | `pytest`               |
| `setup.py`        | `ruff check .`                                | `pytest`               |
| `go.mod`          | `go vet ./...`                                | `go test ./...`        |

Detection order mirrors `detect_test_command` (Cargo → npm → Python → Go).
First match wins. If the override is set, it wins regardless.

**`npx --no-install`** is deliberate: it refuses to download eslint if the
project doesn't already have it, which prevents surprise network calls and
long installs. If eslint isn't installed locally, lint is effectively
skipped for that project.

**`run_checks` behaviour:**

The existing `run_tests(cwd, cmd, timeout) -> TestResult` helper is
renamed to `run_command(cwd, cmd, timeout) -> CommandResult` because it
is no longer tests-specific. `TestResult` → `CommandResult` with the same
variants (`Pass`, `Fail`, `Skipped`, `Timeout`, plus the module-level
`NoRunners` moves onto `CheckOutcome`). Behaviour is identical; this is
a rename-only refactor so the function's purpose matches both callers.

1. If `lint_cmd` is `Some`, call `run_command(cwd, lint_cmd, timeout)`.
   On `Pass` continue; on `Fail { stderr }` record lint failure and
   **skip the test command** (fast-fail); on `Timeout` record "lint
   timed out after Ns" as the stderr; on `Skipped { reason }` record
   the reason silently.
2. If lint passed (or was `None`) and `test_cmd` is `Some`, call
   `run_command(cwd, test_cmd, timeout)`. Record result the same way.
3. If both commands are `None`, return `NoRunners`.
4. Return `Pass` iff both records are `Pass` (or absent). Otherwise
   `Fail { lint_stderr, test_stderr }` with whichever is set.

The module keeps the existing poll-kill loop; no new dependencies.

**`src/settings.rs`:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AutoFixSettings {
    pub enabled: Option<bool>,
    pub trigger: Option<String>,
    pub lint_command: Option<String>,    // NEW
    pub test_command: Option<String>,
    pub max_retries: Option<u32>,
    pub timeout_secs: Option<u64>,
}

pub struct Settings {
    // ...
    #[serde(rename = "autoFixLoop", alias = "autoRollback")]
    pub auto_fix: Option<AutoFixSettings>,
    // ...
}
```

The `alias` ensures existing `"autoRollback": { ... }` blocks in user
settings still deserialise without migration.

**`src/config.rs`:**

- Rename field `auto_rollback` → `auto_fix`
- Parse `lint_command` alongside `test_command`
- Cap `max_retries` at `1..=10` (default `3`, ceiling `10`); values outside
  the range clamp to `3` with a warning to stderr
- Remove the "reserved for future multi-turn loop" warning on `max_retries`

**`src/tui/run.rs`** (around line 4053, the existing rollback block):

Replace the single-shot check with a retry state machine that lives **one
level up** in the agentic loop. Concretely:

- Move `rollback_touched` → `auto_fix_touched` (same semantics, renamed)
- Add `auto_fix_retries_used: u32` initialised to 0 at the top of
  `run_api_task`
- After tool execution, instead of the current block at lines 4053-4125,
  call a new local helper `run_auto_fix_check(...)` that returns one of:
  - `AutoFixAction::Continue` — pass or trigger rules say skip → agentic
    loop continues normally
  - `AutoFixAction::Retry(feedback_msg)` — fail and under cap → append
    `Message { role: User, content: [Text { text: feedback_msg }] }` to
    `messages`, increment `retries_used`, `continue` the outer loop
  - `AutoFixAction::GiveUp(summary)` — fail and at cap → emit SystemMessage
    with `summary`, emit `Done` event with current `messages`, return
- On `Continue` (whether because of pass or skip), clear `auto_fix_touched`
  so the next turn starts fresh
- On `Retry`, also clear `auto_fix_touched` — the retry itself may touch
  the same or different files, and we only care about what the retry
  produces
- **Critical**: the `continue` statement must be `continue` of the outer
  `loop` in `run_api_task`, not the tool-execution `for` loop. That is
  already how the existing agentic flow works; we just re-enter the API
  call at the top of the outer loop.

---

## Interaction with existing systems

| System               | Interaction                                                                             |
|----------------------|-----------------------------------------------------------------------------------------|
| Autonomy modes       | `Autonomous` trigger mode only fires in `auto-edit` or `full-auto` (unchanged)          |
| Budget cap           | Each retry is a real API round → `over_budget()` check in the loop stops retries       |
| Turn limit           | Retries don't bypass `max_turns`; each retry counts as one turn                         |
| Esc cancellation     | Cancel signals propagate normally during retry rounds                                   |
| Context health       | Retries increase context usage; the usual 85% warning fires                             |
| Session save/resume  | Retries leave normal assistant/user turns in history, so replay works unchanged         |
| Hooks (Post/PreToolUse) | Unchanged — hooks fire per-tool as before; auto-fix runs *after* hooks                |
| Permissions          | Unchanged — the model's retry edits go through permission checks like any other edit  |
| `/undo`              | Still works — it's a git-level operation independent of this loop                      |

---

## Config shape (settings.json)

```json
{
  "autoFixLoop": {
    "enabled": true,
    "trigger": "autonomous",
    "lintCommand": null,
    "testCommand": null,
    "maxRetries": 3,
    "timeoutSecs": 60
  }
}
```

**Defaults:**

| Key           | Default      | Bound           |
|---------------|--------------|-----------------|
| `enabled`     | `true`       | bool            |
| `trigger`     | `"autonomous"` | `autonomous` / `always` / `off` |
| `lintCommand` | auto-detect  | string or null  |
| `testCommand` | auto-detect  | string or null  |
| `maxRetries`  | `3`          | clamped to `1..=10` |
| `timeoutSecs` | `60`         | `u64`, `0` = no timeout |

**Backward compatibility:** a settings block under `"autoRollback": {...}`
still deserialises identically. No warning is printed — that would spam
every existing user.

---

## User-visible behaviour

### Success path (pass on first try)

```
[Model edits foo.rs]
[auto-fix] running lint: cargo clippy --all-targets -- -D warnings
[auto-fix] running tests: cargo test
[auto-fix] all checks passed
```

### Success after one retry

```
[Model edits foo.rs]
[auto-fix] running lint: cargo clippy --all-targets -- -D warnings
[auto-fix] lint failed — retry 1/3
[Model edits foo.rs again]
[auto-fix] running lint: cargo clippy --all-targets -- -D warnings
[auto-fix] running tests: cargo test
[auto-fix] all checks passed
```

### Cap reached

```
[Model edits foo.rs]
[auto-fix] lint failed — retry 1/3
[Model edits foo.rs again]
[auto-fix] lint failed — retry 2/3
[Model edits foo.rs again]
[auto-fix] tests failed — retry 3/3
[Model edits foo.rs again]
[auto-fix] cap reached — giving up, working tree left as-is
[auto-fix] Final lint output:
   error: function `foo` is never used ...
[auto-fix] Final test output:
   (none — lint failed)
```

The turn ends with a normal `Done` event so the user can re-prompt or
`/undo`.

### Skipped (autonomy mode is `suggest`)

No auto-fix output at all. Silent skip. Users in suggest mode are already
being asked to approve every edit, so piling on an auto-fix run is
redundant and costly.

### No runners detected

Silent skip. This matches the existing `NoTestRunner` behaviour.

---

## Testing strategy

### Unit tests (`src/autofix.rs`, `#[cfg(test)]`)

- `detect_lint_command_cargo` — Cargo.toml → `cargo clippy ...`
- `detect_lint_command_npm` — package.json → `npx --no-install eslint .`
- `detect_lint_command_python_pyproject` — pyproject.toml → `ruff check .`
- `detect_lint_command_python_setuppy` — setup.py → `ruff check .`
- `detect_lint_command_go` — go.mod → `go vet ./...`
- `detect_lint_command_override_wins` — override always takes precedence
- `detect_lint_command_no_runner` — empty dir → `None`
- `format_feedback_message_both_failed` — rendered output contains anti-cheat clause
- `format_feedback_message_lint_only` — test section reads "(skipped: lint failed)"
- `format_feedback_message_truncates_lint` — 10KB stderr trimmed to 2KB + ellipsis
- `format_feedback_message_truncates_tests` — same for test stderr
- `run_checks_lint_fail_skips_tests` — uses `run_tests` with a synthetic failing lint and a should-never-run test; assert test never ran (via a sentinel file or tmpdir absence)
- `run_checks_lint_pass_tests_fail` — lint passes (e.g. `true`), test fails (e.g. `false`)
- `run_checks_both_pass` — both `true` → `CheckOutcome::Pass`
- `run_checks_no_runners` — both `None` → `NoRunners`
- `should_trigger_autonomy_modes` — existing test, kept, updated for new struct name

### Settings tests (`src/settings.rs`, `#[cfg(test)]`)

- `settings_parses_new_key` — `{"autoFixLoop": {...}}` deserialises
- `settings_parses_legacy_alias` — `{"autoRollback": {...}}` deserialises to the same struct
- `settings_merges_new_and_alias` — if both present, later-loaded wins (existing merge behaviour)

### Config tests (`src/config.rs`, `#[cfg(test)]`)

- `config_clamps_max_retries_too_low` — `max_retries: 0` → `3` with warning
- `config_clamps_max_retries_too_high` — `max_retries: 50` → `3` with warning
- `config_keeps_valid_max_retries` — `max_retries: 5` → `5`
- `config_parses_lint_command` — override populated
- `config_parses_legacy_autoRollback_block` — smoke test the alias wiring

### Integration test (`tests/autofix_loop_integration.rs`, new)

Because the retry state machine lives inside `run_api_task` (a giant async
closure), we test the **helper `run_auto_fix_check`** in isolation:

1. **Extract the helper** into an independently-callable function:
   `pub(crate) fn run_auto_fix_check(...) -> AutoFixAction`
2. **Test 1 — pass:** mock cwd with a `Cargo.toml` containing `fn main(){}`,
   override `lint_command: "true"` and `test_command: "true"`, expect
   `AutoFixAction::Continue`
3. **Test 2 — lint fail under cap:** `lint_command: "false"`, retries_used
   = 0, max_retries = 3, expect `AutoFixAction::Retry(msg)` with `msg`
   containing the anti-cheat clause and "(skipped: lint failed)"
4. **Test 3 — test fail under cap:** `lint_command: "true"`,
   `test_command: "false"`, expect `Retry` with the test failure surfaced
5. **Test 4 — cap reached:** `lint_command: "false"`, retries_used = 3,
   max_retries = 3, expect `AutoFixAction::GiveUp(summary)` with a
   user-friendly summary
6. **Test 5 — trigger skip:** `trigger: Off` → expect `Continue` without
   running any commands (use a sentinel file that the lint command would
   create)
7. **Test 6 — no runners:** empty tmpdir, no overrides → `Continue`

The `run_api_task` wiring itself is exercised by manual QA and the
existing TUI event flow. Unit + integration tests on the pure helper give
us the behaviour guarantees; wiring is a ~20-line inlined call.

---

## Rollout

1. Implementation lands behind `enabled: true` by default in `autonomous`
   trigger mode. Users in `suggest` or `plan-only` see zero behaviour
   change.
2. CHANGELOG entry under Phase 2.
3. README gets a short paragraph under "Our Advantages Over [redacted]" —
   this is a competitive differentiator ([redacted] has no auto-fix loop).
4. No migration script needed. Existing `autoRollback` users silently
   upgrade.

---

## Open questions / deferred

- **`clippy --fix` opt-in**: a `"autoApplyFixes": true` flag could run the
  linter's own auto-fix before falling back to the model. Cheaper but
  muddier blame. Defer until users ask.
- **Per-crate lint scope**: `cargo clippy -p <crate>` when touched files
  are all in one crate. Optimisation only; defer.
- **Structured failure parsing**: parse clippy JSON output and inline just
  the relevant file:line snippets. Nicer context use. Defer until we have
  data on token cost.
- **Web tests / browser checks**: out of scope for Phase 2.
