/// FTS5-powered code search over the RAG index.
///
/// Given a natural language query (or keywords), retrieves the most relevant
/// code chunks ranked by FTS5 relevance score + symbol-kind boost.

use anyhow::Result;

use super::RagDb;

/// A search result from the RAG index.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub file_path: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub language: String,
    pub start_line: i64,
    pub end_line: i64,
    pub content: String,
    /// FTS5 relevance rank (lower = more relevant).
    pub rank: f64,
}

impl SearchResult {
    /// Format this result as context for injection into the system prompt.
    pub fn as_context(&self) -> String {
        format!(
            "# {file}:{start}-{end} ({kind} `{name}`, {lang})\n{content}",
            file = self.file_path,
            start = self.start_line,
            end = self.end_line,
            kind = self.symbol_kind,
            name = self.symbol_name,
            lang = self.language,
            content = self.content,
        )
    }
}

/// Sanitize a query string for FTS5.
/// FTS5 syntax uses quotes, AND, OR, NOT, NEAR etc.
/// We escape user input to prevent syntax errors.
fn sanitize_fts_query(query: &str) -> String {
    // Split into words, quote each, join with space (implicit AND)
    query
        .split_whitespace()
        .filter(|w| w.len() >= 2) // skip very short words
        .map(|w| {
            // Strip FTS5 operators
            let clean: String = w
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            if clean.is_empty() {
                return String::new();
            }
            // Use prefix matching for flexibility
            format!("\"{clean}\"*")
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Search the RAG index for code chunks matching a query.
///
/// Returns up to `limit` results sorted by relevance.
/// The query can be natural language — keywords are extracted and matched
/// against symbol names and code content via FTS5.
pub fn search(db: &RagDb, query: &str, limit: i64) -> Result<Vec<SearchResult>> {
    let fts_query = sanitize_fts_query(query);
    if fts_query.is_empty() {
        return Ok(vec![]);
    }

    let mut stmt = db.conn.prepare(
        "SELECT
            c.file_path,
            c.symbol_name,
            c.symbol_kind,
            c.language,
            c.start_line,
            c.end_line,
            c.content,
            chunks_fts.rank
         FROM chunks_fts
         JOIN code_chunks c ON c.id = chunks_fts.rowid
         WHERE chunks_fts MATCH ?1
         ORDER BY chunks_fts.rank
         LIMIT ?2",
    )?;

    let results = stmt.query_map(rusqlite::params![fts_query, limit], |row| {
        Ok(SearchResult {
            file_path: row.get(0)?,
            symbol_name: row.get(1)?,
            symbol_kind: row.get(2)?,
            language: row.get(3)?,
            start_line: row.get(4)?,
            end_line: row.get(5)?,
            content: row.get(6)?,
            rank: row.get(7)?,
        })
    })?;

    let mut out = Vec::new();
    for r in results {
        if let Ok(r) = r {
            out.push(r);
        }
    }
    Ok(out)
}

/// Search specifically for a symbol by name (exact or prefix match).
/// Useful for "find me the function called X" queries.
pub fn search_symbol(db: &RagDb, name: &str, limit: i64) -> Result<Vec<SearchResult>> {
    let mut stmt = db.conn.prepare(
        "SELECT
            file_path, symbol_name, symbol_kind, language,
            start_line, end_line, content, 0.0 as rank
         FROM code_chunks
         WHERE symbol_name LIKE ?1
         ORDER BY
            CASE WHEN symbol_name = ?2 THEN 0 ELSE 1 END,
            file_path
         LIMIT ?3",
    )?;

    let pattern = format!("{name}%");
    let results = stmt.query_map(rusqlite::params![pattern, name, limit], |row| {
        Ok(SearchResult {
            file_path: row.get(0)?,
            symbol_name: row.get(1)?,
            symbol_kind: row.get(2)?,
            language: row.get(3)?,
            start_line: row.get(4)?,
            end_line: row.get(5)?,
            content: row.get(6)?,
            rank: row.get(7)?,
        })
    })?;

    let mut out = Vec::new();
    for r in results {
        if let Ok(r) = r {
            out.push(r);
        }
    }
    Ok(out)
}

/// Build a context string from search results, suitable for injection
/// into the system prompt or a user message.
///
/// Caps total output at `max_chars` to avoid blowing up the context window.
pub fn build_context(results: &[SearchResult], max_chars: usize) -> String {
    if results.is_empty() {
        return String::new();
    }

    let mut parts = Vec::new();
    let mut total_chars = 0;

    for r in results {
        let chunk = r.as_context();
        if total_chars + chunk.len() > max_chars {
            break;
        }
        total_chars += chunk.len();
        parts.push(chunk);
    }

    if parts.is_empty() {
        return String::new();
    }

    format!(
        "<codebase_context>\n{}\n</codebase_context>",
        parts.join("\n\n")
    )
}
