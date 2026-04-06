/// Tool trait and registry — port of Tool.ts and tools.ts

pub mod agent;
pub mod ask_user;
pub mod bash;
pub mod brief_tool;
pub mod config_tool;
pub mod cron;
pub mod discover_skills;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod glob;
pub mod grep;
pub mod lsp;
pub mod mcp_resources;
pub mod memory;
pub mod multi_edit;
pub mod notebook;
pub mod plan_mode;
pub mod powershell;
pub mod skill_tool;
pub mod send_message;
pub mod sleep;
pub mod team_tools;
pub mod tasks;
pub mod todo;
pub mod tool_search;
pub mod web_browser;
pub mod web_fetch;
pub mod web_search;
pub mod workflow;
pub mod worktree;

use crate::api::types::{ToolDefinition, ToolResultContent};
use anyhow::Result;
pub use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Cache of (path → content-hash) populated by the Read tool so it can
/// emit a compact "unchanged since last read" notice instead of re-sending
/// the full file body on repeat reads. v2.1.86 token-overhead fix.
pub type ReadCache = Arc<Mutex<HashMap<std::path::PathBuf, u64>>>;

pub fn new_read_cache() -> ReadCache {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Permission mode for the session — mirrors PermissionMode in types/permissions.ts
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionMode {
    Default,
    AutoEdit,
    BypassPermissions,
}

/// Minimal tool execution context — mirrors ToolUseContext in Tool.ts
#[derive(Clone)]
pub struct ToolContext {
    pub cwd: std::path::PathBuf,
    pub permission_mode: PermissionMode,
    pub verbose: bool,
    /// Optional channel for streaming live output lines to the TUI during tool execution.
    pub stream_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    /// Optional channel for AskUserQuestion tool to request user input.
    /// Tuple: (question_text, reply_sender).
    pub ask_user_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, oneshot::Sender<String>)>>,
    /// Optional channel for plan mode tools to toggle plan mode.
    pub plan_mode_tx: Option<tokio::sync::mpsc::UnboundedSender<bool>>,
    /// Default shell for the Bash tool ("bash", "powershell", etc.).
    /// None = use $SHELL env var or "bash" as fallback.
    pub default_shell: Option<String>,

    /// If set, Write/Edit tools snapshot the original file here before modifying it.
    /// Set to `~/.claude/sessions/<sid>/snapshots/turn-<n>/` by the run loop.
    pub snapshot_dir: Option<std::path::PathBuf>,

    /// Active sandbox mode for the Bash tool ("strict", "bwrap", "firejail").
    /// None means sandbox is disabled.
    pub sandbox_mode: Option<String>,

    /// Whether bwrap sandbox allows outbound network (passed to bwrap_wrap).
    pub sandbox_allow_network: bool,

    /// Shared Read-tool cache: path → content hash. Lets the Read tool skip
    /// re-emitting a file body that hasn't changed since the last read.
    pub read_cache: Option<ReadCache>,
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("cwd", &self.cwd)
            .field("permission_mode", &self.permission_mode)
            .field("verbose", &self.verbose)
            .finish()
    }
}

impl ToolContext {
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self {
            cwd,
            permission_mode: PermissionMode::Default,
            verbose: false,
            stream_tx: None,
            ask_user_tx: None,
            plan_mode_tx: None,
            default_shell: None,
            snapshot_dir: None,
            sandbox_mode: None,
            sandbox_allow_network: true,
            read_cache: None,
        }
    }

    /// Send a live output line to the TUI (if a stream channel is set).
    pub fn stream_line(&self, line: &str) {
        if let Some(tx) = &self.stream_tx {
            let _ = tx.send(line.to_string());
        }
    }
}

/// Snapshot a file to the ctx.snapshot_dir before it is modified.
/// The snapshot preserves the file at its current state so /rewind can restore it.
/// Silently skips if snapshot_dir is None or the file doesn't exist yet (new file).
pub async fn snapshot_file(ctx: &ToolContext, path: &std::path::Path) {
    let Some(ref snap_dir) = ctx.snapshot_dir else { return };
    if !path.exists() { return; } // new file — nothing to snapshot

    // Compute a relative-ish path for the snapshot filename
    // e.g. /home/user/project/src/main.rs → "home_user_project_src_main.rs"
    let flat_name = path.display().to_string()
        .replace('/', "_")
        .trim_start_matches('_')
        .to_string();

    if let Err(_) = tokio::fs::create_dir_all(snap_dir).await { return; }
    let dest = snap_dir.join(&flat_name);
    let _ = tokio::fs::copy(path, &dest).await;
}

/// Result of a tool execution
#[derive(Debug)]
pub struct ToolOutput {
    pub content: Vec<ToolResultContent>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultContent::text(text)],
            is_error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolResultContent::text(text)],
            is_error: true,
        }
    }
}

/// Directories that Write/Edit tools must never modify.
/// Matches upstream Claude Code protected-directory list.
pub const PROTECTED_DIRS: &[&str] = &[".git", ".husky"];

/// Returns Some(error ToolOutput) if `path` is inside a protected directory.
pub fn check_protected_path(path: &std::path::Path) -> Option<ToolOutput> {
    for &protected in PROTECTED_DIRS {
        if path.components().any(|c| c.as_os_str() == protected) {
            return Some(ToolOutput::error(format!(
                "Modifying files inside '{}' directories is not allowed.",
                protected
            )));
        }
    }
    None
}

/// The core Tool trait — mirrors the Tool interface in Tool.ts
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name sent to the API
    fn name(&self) -> &str;

    /// Human-readable description sent to the API
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool with the given JSON input
    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput>;

    /// Build the ToolDefinition to include in API requests
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: self.input_schema(),
            cache_control: None,
        }
    }
}

pub type DynTool = Arc<dyn Tool>;

/// Build the default tool set.
pub fn default_tools() -> Vec<DynTool> {
    vec![
        Arc::new(bash::BashTool),
        Arc::new(file_read::FileReadTool),
        Arc::new(file_write::FileWriteTool),
        Arc::new(file_edit::FileEditTool),
        Arc::new(multi_edit::MultiEditTool),
        Arc::new(glob::GlobTool),
        Arc::new(grep::GrepTool),
        Arc::new(web_fetch::WebFetchTool),
    ]
}

/// All shared state needed by tools that must also be readable by slash commands.
pub struct SharedToolState {
    pub todo: todo::TodoState,
}

/// Build the full tool set. Returns tools + shared state so slash commands can read it.
pub fn all_tools_with_state(config: &crate::config::Config) -> (Vec<DynTool>, SharedToolState) {
    let mut tools = default_tools();

    tools.push(Arc::new(web_search::WebSearchTool {
        api_key: config.api_key.clone(),
        model: config.model.clone(),
    }));
    tools.push(Arc::new(agent::AgentTool {
        config: config.clone(),
    }));

    // Task management tools (shared registry)
    let registry = tasks::new_registry();
    tools.push(Arc::new(tasks::TaskCreateTool { registry: registry.clone() }));
    tools.push(Arc::new(tasks::TaskGetTool    { registry: registry.clone() }));
    tools.push(Arc::new(tasks::TaskListTool   { registry: registry.clone() }));
    tools.push(Arc::new(tasks::TaskUpdateTool { registry: registry.clone() }));
    tools.push(Arc::new(tasks::TaskStopTool   { registry: registry.clone() }));
    tools.push(Arc::new(tasks::TaskOutputTool { registry: registry.clone() }));

    // Worktree tools (shared state)
    let wt_state = worktree::WorktreeState::default();
    tools.push(Arc::new(worktree::EnterWorktreeTool { state: wt_state.clone() }));
    tools.push(Arc::new(worktree::ExitWorktreeTool  { state: wt_state }));

    // TodoWrite — shared state readable by /tasks command
    let todo_state = todo::new_todo_state();
    tools.push(Arc::new(todo::TodoWriteTool { state: todo_state.clone() }));

    // Interactive / meta tools
    tools.push(Arc::new(ask_user::AskUserQuestionTool));
    tools.push(Arc::new(plan_mode::EnterPlanModeTool));
    tools.push(Arc::new(plan_mode::ExitPlanModeTool));
    tools.push(Arc::new(memory::MemoryReadTool));
    tools.push(Arc::new(memory::MemoryWriteTool));

    // BriefTool — always-on in Rust (no build-time KAIROS flag system)
    tools.push(Arc::new(brief_tool::BriefTool));

    // Agent swarm tools — enabled when CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1
    if send_message::is_agent_swarms_enabled() {
        tools.push(Arc::new(send_message::SendMessageTool));
        tools.push(Arc::new(team_tools::TeamCreateTool));
        tools.push(Arc::new(team_tools::TeamDeleteTool));
    }

    // Simple utilities
    tools.push(Arc::new(sleep::SleepTool));
    tools.push(Arc::new(powershell::PowerShellTool));
    tools.push(Arc::new(web_browser::WebBrowserTool));
    tools.push(Arc::new(lsp::LSPTool));
    tools.push(Arc::new(discover_skills::DiscoverSkillsTool));
    tools.push(Arc::new(skill_tool::SkillTool));
    tools.push(Arc::new(workflow::WorkflowTool));

    // Notebook tools
    tools.push(Arc::new(notebook::NotebookReadTool));
    tools.push(Arc::new(notebook::NotebookEditTool));

    // Config tool
    tools.push(Arc::new(config_tool::ConfigTool { config: config.clone() }));

    // Cron tools (shared store)
    let cron_store: cron::CronStore = Arc::new(std::sync::Mutex::new(cron::load_jobs()));
    tools.push(Arc::new(cron::CronCreateTool { store: cron_store.clone() }));
    tools.push(Arc::new(cron::CronDeleteTool { store: cron_store.clone() }));
    tools.push(Arc::new(cron::CronListTool   { store: cron_store }));

    // ToolSearch — built last so it can include all tool names+descriptions
    let snapshot: Vec<(String, String)> = tools.iter()
        .map(|t| (t.name().to_string(), t.description().to_string()))
        .collect();
    tools.push(Arc::new(tool_search::ToolSearchTool { tools_snapshot: snapshot }));

    let shared = SharedToolState { todo: todo_state };
    (tools, shared)
}

/// Convenience wrapper for callers that don't need the shared state.
pub fn all_tools(config: &crate::config::Config) -> Vec<DynTool> {
    all_tools_with_state(config).0
}

/// Build tools + shared state, then append MCP dynamic tools and resource tools.
pub fn all_tools_with_state_and_mcp(
    config: &crate::config::Config,
    mcp_tools: Vec<DynTool>,
    mcp_clients: Vec<Arc<crate::mcp::client::McpClient>>,
) -> (Vec<DynTool>, SharedToolState) {
    let (mut tools, shared) = all_tools_with_state(config);
    tools.extend(mcp_tools);

    // MCP resource tools (need the client list)
    if !mcp_clients.is_empty() {
        tools.push(Arc::new(mcp_resources::ListMcpResourcesTool {
            clients: mcp_clients.clone(),
        }));
        tools.push(Arc::new(mcp_resources::ReadMcpResourceTool {
            clients: mcp_clients,
        }));
    }

    (tools, shared)
}
