//! File watcher with AI marker scanning.
//!
//! Watches files for changes and scans for action markers (AI:, AGENT:).
//! Integrates with the TUI event loop via AppEvent.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use notify::{RecommendedWatcher, RecursiveMode, Watcher, Event as NotifyEvent, EventKind};
use tokio::sync::mpsc;

/// A marker found in a file.
#[derive(Debug, Clone)]
pub struct Marker {
    pub file: PathBuf,
    pub line: usize,
    pub text: String,
    pub kind: String, // "AI", "AGENT", "TODO", etc.
}

/// Scan file content for action markers.
pub fn scan_markers(content: &str, patterns: &[&str]) -> Vec<Marker> {
    let mut markers = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Strip comment prefixes
        let stripped = trimmed
            .strip_prefix("//")
            .or_else(|| trimmed.strip_prefix('#'))
            .or_else(|| trimmed.strip_prefix("--"))
            .or_else(|| trimmed.strip_prefix("/*"))
            .map(|s| s.trim())
            .unwrap_or("");

        for pattern in patterns {
            if let Some(rest) = stripped.strip_prefix(pattern) {
                markers.push(Marker {
                    file: PathBuf::new(), // Caller fills this in
                    line: line_idx + 1,
                    text: rest.trim().to_string(),
                    kind: pattern.trim_end_matches(':').to_string(),
                });
            }
        }
    }

    markers
}

/// Configuration for the file watcher.
pub struct WatchConfig {
    pub paths: Vec<PathBuf>,
    pub patterns: Vec<String>,     // glob patterns to include
    pub markers: Vec<String>,      // marker patterns to scan for (e.g. "AI:", "AGENT:")
    pub debounce_ms: u64,
    pub rate_limit_ms: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            paths: vec![PathBuf::from(".")],
            patterns: vec!["*.rs".into(), "*.py".into(), "*.ts".into(), "*.js".into()],
            markers: vec!["AI:".into(), "AGENT:".into()],
            debounce_ms: 500,
            rate_limit_ms: 10_000,
        }
    }
}

/// Watch event sent to the TUI event loop.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    FileChanged { path: PathBuf },
    MarkerFound { marker: Marker },
}

/// Start a file watcher. Returns a receiver for watch events.
/// The watcher handle must be kept alive (dropping it stops watching).
pub fn start_watcher(
    config: WatchConfig,
    tx: mpsc::UnboundedSender<WatchEvent>,
) -> notify::Result<RecommendedWatcher> {
    let rate_limit = Duration::from_millis(config.rate_limit_ms);
    let marker_patterns: Vec<String> = config.markers.clone();

    let mut last_trigger = Instant::now() - rate_limit; // Allow immediate first trigger

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<NotifyEvent>| {
        let Ok(event) = res else { return };

        // Only care about modifications and creations
        if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
            return;
        }

        // Rate limit
        let now = Instant::now();
        if now.duration_since(last_trigger) < rate_limit {
            return;
        }

        for path in &event.paths {
            // Skip non-files and gitignored files
            if !path.is_file() { continue; }
            if path.to_string_lossy().contains(".git/") { continue; }

            let _ = tx.send(WatchEvent::FileChanged { path: path.clone() });

            // Scan for markers
            if let Ok(content) = std::fs::read_to_string(path) {
                let pattern_refs: Vec<&str> = marker_patterns.iter().map(|s| s.as_str()).collect();
                let mut markers = scan_markers(&content, &pattern_refs);
                for m in &mut markers {
                    m.file = path.clone();
                }
                for m in markers {
                    let _ = tx.send(WatchEvent::MarkerFound { marker: m });
                }
            }

            last_trigger = now;
        }
    })?;

    for path in &config.paths {
        watcher.watch(path, RecursiveMode::Recursive)?;
    }

    Ok(watcher)
}

// `debounce_ms` and `Path` are part of the public API used in Task 12 (`/watch`
// command). They're not yet read internally — silence the dead-code warnings
// without deleting the fields.
#[allow(dead_code)]
const _DEBOUNCE_USED_IN_TASK_12: fn(&WatchConfig) -> u64 = |c| c.debounce_ms;
#[allow(dead_code)]
const _PATH_USED_IN_API: fn(&Path) -> bool = |p| p.is_file();
