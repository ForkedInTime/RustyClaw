//! Browser automation tools implementing the Tool trait.
//!
//! Each struct wraps a browser action from src/browser/ and exposes it
//! as a tool the AI model can call. Tools share a single BrowserSession
//! so one browser_navigate launches the browser and subsequent clicks/fills
//! operate on the same page.

use crate::browser::{self, BrowserSession};
use crate::tools::{Tool, ToolContext, ToolOutput};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

pub type SharedSession = Arc<Mutex<BrowserSession>>;

/// Ensure the browser is launched. Launches on first use.
/// Each tool holds its own copy of headless + chrome_path (captured at
/// construction from the Config), so no ToolContext fields are needed.
async fn ensure_launched(
    session: &SharedSession,
    headless: bool,
    chrome_path: Option<&str>,
) -> Result<()> {
    let mut s = session.lock().await;
    if !s.is_connected() {
        s.launch(headless, chrome_path).await?;
    }
    Ok(())
}

// ── browser_navigate ────────────────────────────────────────────────────────

pub struct BrowserNavigateTool {
    pub session: SharedSession,
    pub headless: bool,
    pub chrome_path: Option<String>,
}

#[async_trait]
impl Tool for BrowserNavigateTool {
    fn name(&self) -> &str {
        "browser_navigate"
    }
    fn description(&self) -> &str {
        "Navigate the browser to a URL. Launches the browser if not already running. \
         Returns the page title and an accessibility snapshot."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to navigate to" }
            },
            "required": ["url"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        ensure_launched(&self.session, self.headless, self.chrome_path.as_deref()).await?;
        let url = input["url"].as_str().unwrap_or("about:blank");
        let mut session = self.session.lock().await;
        let (title, status) = browser::actions::navigate(&mut session, url).await?;

        // Auto-snapshot after navigation so the model has refs to act on.
        let (tree, refs) = browser::snapshot::take_snapshot(session.client()?).await?;
        session.set_refs(refs);

        Ok(ToolOutput::success(format!(
            "Navigated to: {url}\nTitle: {title}\nStatus: {status}\n\nAccessibility snapshot:\n{tree}"
        )))
    }
}

// ── browser_snapshot ────────────────────────────────────────────────────────

pub struct BrowserSnapshotTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserSnapshotTool {
    fn name(&self) -> &str {
        "browser_snapshot"
    }
    fn description(&self) -> &str {
        "Capture a fresh accessibility snapshot of the current page. \
         Returns a text tree with @eN refs you can pass to browser_click/browser_fill/etc."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let mut session = self.session.lock().await;
        let (tree, refs) = browser::snapshot::take_snapshot(session.client()?).await?;
        session.set_refs(refs);
        Ok(ToolOutput::success(tree))
    }
}

// ── browser_click ───────────────────────────────────────────────────────────

pub struct BrowserClickTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserClickTool {
    fn name(&self) -> &str {
        "browser_click"
    }
    fn description(&self) -> &str {
        "Click an element identified by an @eN ref from the latest accessibility snapshot. \
         Auto-snapshots afterwards so the updated page is visible."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "Element ref from the last snapshot, e.g. \"@e3\""
                }
            },
            "required": ["ref"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let element_ref = input["ref"].as_str().unwrap_or("");
        let mut session = self.session.lock().await;
        // Force early error if the browser is not connected.
        let _ = session.client()?;
        let result = browser::actions::click(&mut session, element_ref).await?;

        // Auto-snapshot after click so the model's next action sees the post-interaction refs.
        let (tree, refs) = browser::snapshot::take_snapshot(session.client()?).await?;
        session.set_refs(refs);

        Ok(ToolOutput::success(format!(
            "{result}\n\nUpdated snapshot:\n{tree}"
        )))
    }
}

// ── browser_fill ────────────────────────────────────────────────────────────

pub struct BrowserFillTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserFillTool {
    fn name(&self) -> &str {
        "browser_fill"
    }
    fn description(&self) -> &str {
        "Fill a text input / textarea / contenteditable element identified by an @eN ref. \
         Clears the existing value first."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "ref":   { "type": "string", "description": "Element ref from the last snapshot" },
                "value": { "type": "string", "description": "Text to type into the element" }
            },
            "required": ["ref", "value"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let element_ref = input["ref"].as_str().unwrap_or("");
        let value = input["value"].as_str().unwrap_or("");
        let mut session = self.session.lock().await;
        let _ = session.client()?;
        let result = browser::actions::fill(&mut session, element_ref, value).await?;
        Ok(ToolOutput::success(result))
    }
}

// ── browser_screenshot ──────────────────────────────────────────────────────

pub struct BrowserScreenshotTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserScreenshotTool {
    fn name(&self) -> &str {
        "browser_screenshot"
    }
    fn description(&self) -> &str {
        "Capture a PNG screenshot of the current page. Saves to a temp file and returns the path. \
         Pass full_page=true for a full-document capture."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "full_page": {
                    "type": "boolean",
                    "description": "Capture the entire scroll height instead of just the viewport",
                    "default": false
                }
            },
            "required": []
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let full_page = input["full_page"].as_bool().unwrap_or(false);
        let session = self.session.lock().await;
        let _ = session.client()?;
        let b64 = browser::actions::screenshot(&session, full_page).await?;
        drop(session);

        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(&b64)?;
        let path = std::env::temp_dir().join("rustyclaw_screenshot.png");
        tokio::fs::write(&path, &bytes).await?;

        Ok(ToolOutput::success(format!(
            "Screenshot saved to {}\n({} bytes, base64 length: {})",
            path.display(),
            bytes.len(),
            b64.len()
        )))
    }
}

// ── browser_get_text ────────────────────────────────────────────────────────

pub struct BrowserGetTextTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserGetTextTool {
    fn name(&self) -> &str {
        "browser_get_text"
    }
    fn description(&self) -> &str {
        "Read the inner text / textContent of an element identified by an @eN ref."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "ref": { "type": "string", "description": "Element ref from the last snapshot" }
            },
            "required": ["ref"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let element_ref = input["ref"].as_str().unwrap_or("");
        let mut session = self.session.lock().await;
        let _ = session.client()?;
        let text = browser::actions::get_text(&mut session, element_ref).await?;
        Ok(ToolOutput::success(text))
    }
}

// ── browser_press_key ───────────────────────────────────────────────────────

pub struct BrowserPressKeyTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserPressKeyTool {
    fn name(&self) -> &str {
        "browser_press_key"
    }
    fn description(&self) -> &str {
        "Send a single key press to the page (e.g. \"Enter\", \"Tab\", \"Escape\", or a single character)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "key": {
                    "type": "string",
                    "description": "Key name — \"Enter\", \"Tab\", \"Escape\", \"Backspace\", \"Space\", or a single character"
                }
            },
            "required": ["key"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let key = input["key"].as_str().unwrap_or("");
        let session = self.session.lock().await;
        let _ = session.client()?;
        let result = browser::actions::press_key(&session, key).await?;
        Ok(ToolOutput::success(result))
    }
}

// ── browser_wait ────────────────────────────────────────────────────────────

pub struct BrowserWaitTool {
    pub session: SharedSession,
}

#[async_trait]
impl Tool for BrowserWaitTool {
    fn name(&self) -> &str {
        "browser_wait"
    }
    fn description(&self) -> &str {
        "Wait until a CSS selector matches an element on the page, or until the timeout elapses."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "selector":   { "type": "string", "description": "CSS selector to wait for" },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (default 30000)",
                    "default": 30000
                }
            },
            "required": ["selector"]
        })
    }
    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let selector = input["selector"].as_str().unwrap_or("");
        let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(30_000);
        let session = self.session.lock().await;
        let _ = session.client()?;
        let result = browser::actions::wait_for(&session, selector, timeout_ms).await?;
        Ok(ToolOutput::success(result))
    }
}
