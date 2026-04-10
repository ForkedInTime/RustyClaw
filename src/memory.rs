/// Persistent project memory — stores decisions, preferences, patterns, and
/// contextual notes in the same SQLite database used by the RAG indexer.
///
/// Each memory is a keyed (key, value) pair with a category and source.
/// FTS5 full-text search allows fuzzy retrieval; `build_context()` formats
/// the top-N items for injection into the system prompt.
///
/// Database location: `<cwd>/.claude/rag.db` (shared with RAG).

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::fmt;
use std::path::Path;
use tracing::debug;

use crate::rag::RagDb;

// ── Category ──────────────────────────────────────────────────────────────────

/// Memory category — used for filtering and display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Decision,
    Preference,
    Pattern,
    Context,
    Custom(String),
}

impl Category {
    pub fn as_str(&self) -> &str {
        match self {
            Category::Decision   => "decision",
            Category::Preference => "preference",
            Category::Pattern    => "pattern",
            Category::Context    => "context",
            Category::Custom(s)  => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "decision"   => Category::Decision,
            "preference" => Category::Preference,
            "pattern"    => Category::Pattern,
            "context"    => Category::Context,
            other        => Category::Custom(other.to_string()),
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Memory ────────────────────────────────────────────────────────────────────

/// A single memory entry.
#[derive(Debug, Clone)]
pub struct Memory {
    pub id:         i64,
    pub key:        String,
    pub value:      String,
    pub category:   Category,
    pub source:     String,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── MemoryStore ───────────────────────────────────────────────────────────────

/// Wraps the project's SQLite connection for memory CRUD operations.
pub struct MemoryStore {
    conn: Connection,
}

impl MemoryStore {
    /// Open (or create) the memory store backed by the shared RAG database.
    pub fn open(cwd: &Path) -> Result<Self> {
        // RagDb::open ensures the schema (including memory tables) is created.
        let rag = RagDb::open(cwd)?;
        // Transfer ownership of the connection out of RagDb.
        // We own the Connection; RagDb drops cleanly.
        let conn = rag.conn;
        debug!("MemoryStore opened (shared rag.db)");
        Ok(Self { conn })
    }

    // ── Write ─────────────────────────────────────────────────────────────────

    /// Insert or update a memory entry by key.
    /// If the key already exists, the value, category, source, and updated_at
    /// are all replaced (upsert semantics).
    pub fn add(&self, key: &str, value: &str, category: Category, source: &str) -> Result<()> {
        // Check whether this key exists so we can choose INSERT vs UPDATE.
        // Using OR REPLACE would reset the id and re-fire the INSERT trigger,
        // breaking the FTS delete trigger. Manual upsert is safer.
        let exists: bool = self.conn.query_row(
            "SELECT COUNT(*) FROM memory WHERE key = ?1",
            [key],
            |r| r.get::<_, i64>(0),
        )? > 0;

        if exists {
            self.conn.execute(
                "UPDATE memory SET value=?1, category=?2, source=?3, updated_at=unixepoch() WHERE key=?4",
                rusqlite::params![value, category.as_str(), source, key],
            )?;
        } else {
            self.conn.execute(
                "INSERT INTO memory (key, value, category, source) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![key, value, category.as_str(), source],
            )?;
        }
        Ok(())
    }

    /// Auto-add a memory, skipping if a near-duplicate already exists.
    ///
    /// Uses word-overlap ratio: if any existing value shares > 0.8 of its words
    /// with `text`, the entry is considered a duplicate and skipped.
    ///
    /// Returns `true` if a new memory was stored, `false` if skipped.
    pub fn add_auto(&self, text: &str, source: &str) -> Result<bool> {
        // Check for near-duplicates against all existing values.
        let mut stmt = self.conn.prepare("SELECT value FROM memory")?;
        let existing_values: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let text_words: std::collections::HashSet<&str> =
            text.split_whitespace().collect();

        for existing in &existing_values {
            let existing_words: std::collections::HashSet<&str> =
                existing.split_whitespace().collect();
            if existing_words.is_empty() {
                continue;
            }
            let intersection = text_words.intersection(&existing_words).count();
            let union = text_words.union(&existing_words).count();
            if union == 0 {
                continue;
            }
            let overlap = intersection as f64 / union as f64;
            if overlap > 0.8 {
                debug!("add_auto: skipping near-duplicate (overlap={:.2})", overlap);
                return Ok(false);
            }
        }

        // Generate a key from the first few words.
        let key = text
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("_")
            .to_lowercase();
        let key = sanitize_key(&key);

        let cat = auto_categorize(text);
        self.add(&key, text, cat, source)?;
        Ok(true)
    }

    // ── Read ──────────────────────────────────────────────────────────────────

    /// Return all memories, optionally filtered by category.
    /// Ordered by `updated_at DESC` (most recently touched first).
    pub fn list(&self, category: Option<Category>) -> Result<Vec<Memory>> {
        let rows = if let Some(cat) = category {
            let mut stmt = self.conn.prepare(
                "SELECT id, key, value, category, source, created_at, updated_at
                 FROM memory WHERE category = ?1 ORDER BY updated_at DESC",
            )?;
            stmt.query_map([cat.as_str()], row_to_memory)?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, key, value, category, source, created_at, updated_at
                 FROM memory ORDER BY updated_at DESC",
            )?;
            stmt.query_map([], row_to_memory)?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(rows)
    }

    /// FTS5 search over key + value. Returns up to `limit` results.
    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<Memory>> {
        let fts_query = crate::rag::search::sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }

        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.key, m.value, m.category, m.source, m.created_at, m.updated_at
             FROM memory_fts
             JOIN memory m ON m.id = memory_fts.rowid
             WHERE memory_fts MATCH ?1
             ORDER BY memory_fts.rank
             LIMIT ?2",
        )?;

        let results = stmt
            .query_map(rusqlite::params![fts_query, limit], row_to_memory)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Total memory count.
    pub fn count(&self) -> Result<i64> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memory",
            [],
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// Build a context string from the top `n` most-recent memories for
    /// injection into the system prompt.
    ///
    /// Format:
    /// ```text
    /// ## Project Memory
    /// - [decision] We use JWT for auth (2026-04-09)
    /// - [preference] Prefer functional React components (2026-04-09)
    /// ```
    ///
    /// Returns an empty string if no memories exist.
    pub fn build_context(&self, n: usize) -> Result<String> {
        let memories = self.list(None)?;
        if memories.is_empty() {
            return Ok(String::new());
        }

        let lines: Vec<String> = memories
            .iter()
            .take(n)
            .map(|m| {
                let date = format_unix_date(m.updated_at);
                format!("- [{}] {} ({})", m.category, m.value, date)
            })
            .collect();

        Ok(format!("## Project Memory\n{}", lines.join("\n")))
    }

    // ── Delete ────────────────────────────────────────────────────────────────

    /// Remove a memory by key. No-op if the key does not exist.
    pub fn forget(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM memory WHERE key = ?1", [key])
            .context("Failed to delete memory entry")?;
        Ok(())
    }

    /// Delete all memories. Rebuilds the FTS index.
    pub fn clear_all(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM memory;
             INSERT INTO memory_fts(memory_fts) VALUES ('rebuild');",
        )?;
        Ok(())
    }
}

// ── Auto-capture ──────────────────────────────────────────────────────────────

/// Scan an assistant response for signal phrases and return candidate memory
/// strings. Callers can then pass each candidate to `MemoryStore::add_auto`.
pub fn auto_capture_memories(response: &str) -> Vec<String> {
    // Signal phrases that indicate a notable decision or preference.
    let signals = [
        "let's use",
        "we'll use",
        "decided to",
        "we decided",
        "going to use",
        "i'll use",
        "we should use",
        "we are using",
        "always use",
        "never use",
        "prefer to",
        "we prefer",
    ];

    let mut candidates = Vec::new();
    for line in response.lines() {
        let lower = line.to_lowercase();
        if signals.iter().any(|s| lower.contains(s)) {
            let trimmed = line.trim().to_string();
            if trimmed.len() >= 20 && trimmed.len() <= 300 {
                candidates.push(trimmed);
            }
        }
    }
    candidates
}

// ── auto_categorize ───────────────────────────────────────────────────────────

/// Classify text into a category using keyword heuristics.
pub fn auto_categorize(text: &str) -> Category {
    let lower = text.to_lowercase();

    let decision_signals = [
        "decided", "decision", "chose", "chosen", "let's use", "we'll use",
        "going to use", "will use", "selected", "opted", "adopted",
    ];
    let preference_signals = [
        "prefer", "prefers", "preferred", "likes", "always", "never",
        "want", "wants", "favor", "favors",
    ];
    let pattern_signals = [
        "typically", "usually", "pattern", "convention", "habit",
        "tends to", "tends", "often", "regularly", "approach",
    ];

    if decision_signals.iter().any(|s| lower.contains(s)) {
        Category::Decision
    } else if preference_signals.iter().any(|s| lower.contains(s)) {
        Category::Preference
    } else if pattern_signals.iter().any(|s| lower.contains(s)) {
        Category::Pattern
    } else {
        Category::Context
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    let cat_str: String = row.get(3)?;
    Ok(Memory {
        id:         row.get(0)?,
        key:        row.get(1)?,
        value:      row.get(2)?,
        category:   Category::from_str(&cat_str),
        source:     row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

/// Format a Unix timestamp as YYYY-MM-DD (UTC, no-std).
fn format_unix_date(unix: i64) -> String {
    // Simple calculation — days since 1970-01-01.
    let secs = unix.max(0) as u64;
    let days_since_epoch = secs / 86400;

    // Gregorian calendar: algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Strip characters that would break a SQLite key used as a column value.
fn sanitize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .take(64)
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make() -> (TempDir, MemoryStore) {
        let tmp = TempDir::new().unwrap();
        let s = MemoryStore::open(tmp.path()).unwrap();
        (tmp, s)
    }

    #[test]
    fn test_format_unix_date() {
        // 2025-04-09 00:00:00 UTC = 1744156800
        assert_eq!(format_unix_date(1744156800), "2025-04-09");
        // 2026-04-09 00:00:00 UTC = 1775692800
        assert_eq!(format_unix_date(1775692800), "2026-04-09");
        // epoch
        assert_eq!(format_unix_date(0), "1970-01-01");
    }

    #[test]
    fn test_sanitize_key() {
        assert_eq!(sanitize_key("hello world!"), "helloworld");
        assert_eq!(sanitize_key("foo_bar_42"), "foo_bar_42");
    }

    #[test]
    fn test_category_roundtrip() {
        assert_eq!(Category::from_str("decision").as_str(), "decision");
        assert_eq!(Category::from_str("custom_val").as_str(), "custom_val");
    }

    #[test]
    fn test_auto_capture_memories() {
        let response = "We decided to use PostgreSQL as the primary database.\n\
                        Also, let's use Tokio for async runtime.\n\
                        This line has no signal phrase.";
        let candidates = auto_capture_memories(response);
        assert_eq!(candidates.len(), 2);
        assert!(candidates[0].contains("PostgreSQL"));
        assert!(candidates[1].contains("Tokio"));
    }

    #[test]
    fn test_upsert_via_add() {
        let (_tmp, s) = make();
        s.add("auth", "jwt", Category::Decision, "user").unwrap();
        s.add("auth", "oauth2", Category::Decision, "user").unwrap();
        assert_eq!(s.count().unwrap(), 1);
        let all = s.list(None).unwrap();
        assert_eq!(all[0].value, "oauth2");
    }
}
