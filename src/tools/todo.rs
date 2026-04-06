/// TodoWrite tool — port of TodoWriteTool.
///
/// Claude uses this to maintain a structured todo/checklist during a session.
/// The list is stored in a shared Arc<Mutex<>> so the /tasks command can read it.

use super::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TodoStatus {
    Pending,
    #[serde(rename = "in_progress")]
    InProgress,
    Completed,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending    => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Completed  => write!(f, "completed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TodoPriority {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for TodoPriority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoPriority::Low    => write!(f, "low"),
            TodoPriority::Medium => write!(f, "medium"),
            TodoPriority::High   => write!(f, "high"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
}

/// Shared todo state — passed to TodoWriteTool at construction and readable by /tasks.
pub type TodoState = Arc<Mutex<Vec<TodoItem>>>;

pub fn new_todo_state() -> TodoState {
    Arc::new(Mutex::new(Vec::new()))
}

// ── Tool ──────────────────────────────────────────────────────────────────────

pub struct TodoWriteTool {
    pub state: TodoState,
}

#[derive(Deserialize)]
struct TodoWriteInput {
    todos: Vec<TodoItemInput>,
}

#[derive(Deserialize)]
struct TodoItemInput {
    id: String,
    content: String,
    status: String,
    priority: String,
}

#[async_trait]
impl Tool for TodoWriteTool {
    fn name(&self) -> &str { "TodoWrite" }

    fn description(&self) -> &str {
        "Create and manage a structured task list for the current coding session. \
         Use this to track progress on complex multi-step tasks. \
         Update statuses as you work: pending → in_progress → completed."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "The complete updated todo list",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id":       { "type": "string", "description": "Unique identifier (e.g. '1', '2', 'task-auth')" },
                            "content":  { "type": "string", "description": "Task description" },
                            "status":   { "type": "string", "enum": ["pending", "in_progress", "completed"], "description": "Current status" },
                            "priority": { "type": "string", "enum": ["high", "medium", "low"], "description": "Task priority" }
                        },
                        "required": ["id", "content", "status", "priority"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let parsed: TodoWriteInput = serde_json::from_value(input)
            .map_err(|e| anyhow::anyhow!("Invalid TodoWrite input: {e}"))?;

        let new_todos: Vec<TodoItem> = parsed.todos.into_iter().map(|t| {
            let status = match t.status.as_str() {
                "in_progress" => TodoStatus::InProgress,
                "completed"   => TodoStatus::Completed,
                _             => TodoStatus::Pending,
            };
            let priority = match t.priority.as_str() {
                "high"   => TodoPriority::High,
                "low"    => TodoPriority::Low,
                _        => TodoPriority::Medium,
            };
            TodoItem { id: t.id, content: t.content, status, priority }
        }).collect();

        let summary = {
            let pending    = new_todos.iter().filter(|t| t.status == TodoStatus::Pending).count();
            let in_progress = new_todos.iter().filter(|t| t.status == TodoStatus::InProgress).count();
            let completed  = new_todos.iter().filter(|t| t.status == TodoStatus::Completed).count();
            format!(
                "Todo list updated: {} items ({} pending, {} in progress, {} completed)",
                new_todos.len(), pending, in_progress, completed
            )
        };

        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = new_todos;

        Ok(ToolOutput::success(summary))
    }
}
