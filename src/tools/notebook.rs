/// NotebookEditTool — port of notebook.ts
/// Read and edit Jupyter notebook (.ipynb) cells.

use super::{async_trait, Tool, ToolContext, ToolOutput};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub struct NotebookReadTool;
pub struct NotebookEditTool;

// ── Notebook JSON types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct Notebook {
    cells: Vec<Cell>,
    #[serde(flatten)]
    extra: std::collections::HashMap<String, Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Cell {
    id: Option<String>,
    cell_type: String,
    source: CellSource,
    #[serde(flatten)]
    extra: std::collections::HashMap<String, Value>,
}

/// source can be a string or an array of strings in .ipynb
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum CellSource {
    Lines(Vec<String>),
    Text(String),
}

impl CellSource {
    fn to_string(&self) -> String {
        match self {
            CellSource::Lines(v) => v.join(""),
            CellSource::Text(s) => s.clone(),
        }
    }

    fn from_str(s: &str) -> Self {
        // Store as lines array (each line ends with \n except the last)
        let lines: Vec<String> = if s.is_empty() {
            vec![]
        } else {
            let mut lines: Vec<String> = s.lines().map(|l| l.to_string()).collect();
            // Add \n to all but the last line
            for i in 0..lines.len().saturating_sub(1) {
                lines[i].push('\n');
            }
            lines
        };
        CellSource::Lines(lines)
    }
}

// ── NotebookRead ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ReadInput {
    notebook_path: String,
}

#[async_trait]
impl Tool for NotebookReadTool {
    fn name(&self) -> &str { "NotebookRead" }

    fn description(&self) -> &str {
        "Read the contents of a Jupyter notebook (.ipynb) file. Returns each cell's id, \
        type (code/markdown), and source. Outputs from previous executions are not included."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "notebook_path": {
                    "type": "string",
                    "description": "Path to the .ipynb notebook file"
                }
            },
            "required": ["notebook_path"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let input: ReadInput = serde_json::from_value(input)?;
        let path = resolve_path(&ctx.cwd, &input.notebook_path);

        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| anyhow!("Cannot read {}: {}", path.display(), e))?;

        let notebook: Notebook = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Invalid notebook JSON: {e}"))?;

        let mut out = String::new();
        for (i, cell) in notebook.cells.iter().enumerate() {
            let id = cell.id.as_deref().unwrap_or("(no id)");
            let source = cell.source.to_string();
            out.push_str(&format!(
                "Cell {} [{}] id={}\n{}\n\n",
                i + 1,
                cell.cell_type,
                id,
                source
            ));
        }

        if out.is_empty() {
            out = "Notebook has no cells.".to_string();
        }

        Ok(ToolOutput::success(out))
    }
}

// ── NotebookEdit ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EditInput {
    notebook_path: String,
    /// Cell id to target (for replace/delete). Required for replace and delete.
    cell_id: Option<String>,
    /// New source content (for replace/insert).
    new_source: Option<String>,
    /// Cell type for insert: "code" or "markdown" (default "code")
    cell_type: Option<String>,
    /// Operation: "replace" | "insert_before" | "insert_after" | "delete"
    edit_mode: String,
}

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str { "NotebookEdit" }

    fn description(&self) -> &str {
        "Edit a Jupyter notebook cell. Supports replace, insert_before, insert_after, and delete. \
        Use NotebookRead first to get cell ids."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "notebook_path": {
                    "type": "string",
                    "description": "Path to the .ipynb notebook file"
                },
                "cell_id": {
                    "type": "string",
                    "description": "ID of the target cell (required for replace, insert_before, insert_after, delete)"
                },
                "new_source": {
                    "type": "string",
                    "description": "New source content for replace or insert operations"
                },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "Cell type for insert operations (default: code)"
                },
                "edit_mode": {
                    "type": "string",
                    "enum": ["replace", "insert_before", "insert_after", "delete"],
                    "description": "Operation to perform"
                }
            },
            "required": ["notebook_path", "edit_mode"]
        })
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let input: EditInput = serde_json::from_value(input)?;
        let path = resolve_path(&ctx.cwd, &input.notebook_path);

        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| anyhow!("Cannot read {}: {}", path.display(), e))?;

        let mut notebook: Notebook = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Invalid notebook JSON: {e}"))?;

        match input.edit_mode.as_str() {
            "replace" => {
                let cell_id = input.cell_id.as_deref()
                    .ok_or_else(|| anyhow!("cell_id is required for replace"))?;
                let new_source = input.new_source.as_deref()
                    .ok_or_else(|| anyhow!("new_source is required for replace"))?;

                let cell = notebook.cells.iter_mut()
                    .find(|c| c.id.as_deref() == Some(cell_id))
                    .ok_or_else(|| anyhow!("Cell not found: {cell_id}"))?;

                cell.source = CellSource::from_str(new_source);
            }
            "insert_before" | "insert_after" => {
                let cell_id = input.cell_id.as_deref()
                    .ok_or_else(|| anyhow!("cell_id is required for insert"))?;
                let new_source = input.new_source.as_deref()
                    .ok_or_else(|| anyhow!("new_source is required for insert"))?;
                let cell_type = input.cell_type.as_deref().unwrap_or("code").to_string();

                let idx = notebook.cells.iter()
                    .position(|c| c.id.as_deref() == Some(cell_id))
                    .ok_or_else(|| anyhow!("Cell not found: {cell_id}"))?;

                let new_cell = Cell {
                    id: Some(uuid::Uuid::new_v4().to_string().chars().take(8).collect()),
                    cell_type,
                    source: CellSource::from_str(new_source),
                    extra: Default::default(),
                };

                let insert_at = if input.edit_mode == "insert_before" { idx } else { idx + 1 };
                notebook.cells.insert(insert_at, new_cell);
            }
            "delete" => {
                let cell_id = input.cell_id.as_deref()
                    .ok_or_else(|| anyhow!("cell_id is required for delete"))?;

                let idx = notebook.cells.iter()
                    .position(|c| c.id.as_deref() == Some(cell_id))
                    .ok_or_else(|| anyhow!("Cell not found: {cell_id}"))?;

                notebook.cells.remove(idx);
            }
            other => return Ok(ToolOutput::error(format!("Unknown edit_mode: {other}"))),
        }

        let updated = serde_json::to_string_pretty(&notebook)?;
        tokio::fs::write(&path, updated).await
            .map_err(|e| anyhow!("Cannot write {}: {}", path.display(), e))?;

        Ok(ToolOutput::success(format!(
            "Notebook {} updated successfully.",
            path.display()
        )))
    }
}

fn resolve_path(cwd: &std::path::Path, p: &str) -> std::path::PathBuf {
    let expanded = if p.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            home.join(&p[2..])
        } else {
            std::path::PathBuf::from(p)
        }
    } else {
        std::path::PathBuf::from(p)
    };

    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}
