//! Browser automation via Chrome DevTools Protocol.
pub mod cdp;
pub mod snapshot;
pub mod actions;
pub mod element;
pub mod extraction;
pub mod planner;
pub mod loop_detector;

use anyhow::{Result, bail};
use cdp::CdpClient;
use std::collections::HashMap;
use std::path::PathBuf;

/// Active browser session — owns the CDP connection and page state.
pub struct BrowserSession {
    client: Option<CdpClient>,
    /// Element ref map: @e1 -> backend DOM node ID
    refs: HashMap<String, i64>,
    /// Current page URL
    pub current_url: String,
    /// Current page title
    pub current_title: String,
}

impl Default for BrowserSession {
    fn default() -> Self {
        Self {
            client: None,
            refs: HashMap::new(),
            current_url: String::new(),
            current_title: String::new(),
        }
    }
}

impl BrowserSession {
    pub fn is_connected(&self) -> bool {
        self.client.is_some()
    }

    pub fn ref_map(&self) -> &HashMap<String, i64> {
        &self.refs
    }

    pub fn client(&self) -> Result<&CdpClient> {
        self.client.as_ref().ok_or_else(|| anyhow::anyhow!("Browser not connected. Use /browser or browser_navigate first."))
    }

    /// Launch Chrome and connect via CDP.
    pub async fn launch(&mut self, headless: bool, chrome_path: Option<&str>) -> Result<()> {
        if self.client.is_some() {
            return Ok(());
        }

        let chrome = match chrome_path {
            Some(p) => PathBuf::from(p),
            None => find_chrome().ok_or_else(|| anyhow::anyhow!(
                "Chrome/Chromium not found. Install Chrome or set browserChromePath in settings.json"
            ))?,
        };

        let user_data = tempfile::tempdir()?;
        let port = find_free_port().await;

        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={}", user_data.path().display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-extensions".to_string(),
        ];
        if headless {
            args.push("--headless=new".to_string());
        }
        args.push("about:blank".to_string());

        let _child = tokio::process::Command::new(&chrome)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to launch Chrome at {}: {e}", chrome.display()))?;

        let ws_url = poll_cdp_endpoint(port).await?;
        self.client = Some(CdpClient::connect(&ws_url).await?);
        Ok(())
    }

    /// Connect to an existing CDP endpoint.
    pub async fn connect(&mut self, endpoint: &str) -> Result<()> {
        self.client = Some(CdpClient::connect(endpoint).await?);
        Ok(())
    }

    /// Close the browser session.
    pub fn close(&mut self) {
        self.client = None;
        self.refs.clear();
        self.current_url.clear();
        self.current_title.clear();
    }

    /// Update ref map (called after each snapshot).
    pub fn set_refs(&mut self, refs: HashMap<String, i64>) {
        self.refs = refs;
    }

    /// Resolve an @eN ref to a backend node ID.
    pub fn resolve_ref(&self, r: &str) -> Result<i64> {
        let key = if r.starts_with('@') { r.to_string() } else { format!("@{r}") };
        self.refs.get(&key)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Element ref '{key}' not found. Run browser_snapshot first."))
    }
}

/// Search common Chrome/Chromium binary locations.
pub fn find_chrome() -> Option<PathBuf> {
    let candidates = [
        "google-chrome-stable",
        "google-chrome",
        "chromium-browser",
        "chromium",
        "chrome",
    ];
    for name in &candidates {
        if let Ok(output) = std::process::Command::new("which")
            .arg(name)
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    let known = [
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome",
        "/snap/bin/chromium",
    ];
    for path in &known {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Find a free TCP port for Chrome's debugging port.
async fn find_free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Poll Chrome's /json/version endpoint until the WebSocket debugger URL is available.
async fn poll_cdp_endpoint(port: u16) -> Result<String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest::Client::new();

    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Ok(resp) = client.get(&url).send().await {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(ws) = json["webSocketDebuggerUrl"].as_str() {
                    return Ok(ws.to_string());
                }
            }
        }
    }
    bail!("Chrome did not start within 6 seconds on port {port}")
}
