/// Local codebase RAG — tree-sitter AST indexing + SQLite FTS5 search.
///
/// Indexes the project's source code into per-symbol chunks, stored in a local
/// SQLite database.  When the agent needs context, FTS5 retrieves the most
/// relevant code spans — so the LLM sees surgical context instead of whole files.
///
/// Index location: `<project>/.claude/rag.db`
///
/// This is the feature [redacted] charges $20/month for.  We do it locally, for free,
/// in a single binary with zero external dependencies.

pub mod indexer;
pub mod search;

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::debug;

/// The RAG database — owns a SQLite connection with FTS5 tables.
pub struct RagDb {
    pub conn: Connection,
    pub db_path: PathBuf,
}

impl RagDb {
    /// Open (or create) the RAG database for a project.
    /// Stored at `<cwd>/.claude/rag.db`.
    pub fn open(cwd: &Path) -> Result<Self> {
        let claude_dir = cwd.join(".claude");
        std::fs::create_dir_all(&claude_dir)
            .context("Failed to create .claude directory for RAG index")?;

        let db_path = claude_dir.join("rag.db");
        let conn = Connection::open(&db_path)
            .context("Failed to open RAG database")?;

        // Performance: WAL mode + relaxed sync for indexing speed
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -8000;",  // 8MB cache
        )?;

        // Create tables if they don't exist
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS code_chunks (
                id          INTEGER PRIMARY KEY,
                file_path   TEXT NOT NULL,
                symbol_name TEXT NOT NULL,
                symbol_kind TEXT NOT NULL,
                language    TEXT NOT NULL,
                start_line  INTEGER NOT NULL,
                end_line    INTEGER NOT NULL,
                content     TEXT NOT NULL,
                mtime       INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_chunks_file ON code_chunks(file_path);
            CREATE INDEX IF NOT EXISTS idx_chunks_symbol ON code_chunks(symbol_name);

            CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
                symbol_name,
                content,
                content=code_chunks,
                content_rowid=id,
                tokenize='porter unicode61'
            );

            -- Triggers to keep FTS in sync with the content table
            CREATE TRIGGER IF NOT EXISTS chunks_ai AFTER INSERT ON code_chunks BEGIN
                INSERT INTO chunks_fts(rowid, symbol_name, content)
                VALUES (new.id, new.symbol_name, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS chunks_ad AFTER DELETE ON code_chunks BEGIN
                INSERT INTO chunks_fts(chunks_fts, rowid, symbol_name, content)
                VALUES ('delete', old.id, old.symbol_name, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS chunks_au AFTER UPDATE ON code_chunks BEGIN
                INSERT INTO chunks_fts(chunks_fts, rowid, symbol_name, content)
                VALUES ('delete', old.id, old.symbol_name, old.content);
                INSERT INTO chunks_fts(rowid, symbol_name, content)
                VALUES (new.id, new.symbol_name, new.content);
            END;

            -- Metadata table for tracking index state
            CREATE TABLE IF NOT EXISTS rag_meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- ── Memory tables ──────────────────────────────────────────────────
            CREATE TABLE IF NOT EXISTS memory (
                id          INTEGER PRIMARY KEY,
                key         TEXT NOT NULL UNIQUE,
                value       TEXT NOT NULL,
                category    TEXT NOT NULL DEFAULT 'context',
                source      TEXT NOT NULL DEFAULT 'user',
                created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
                updated_at  INTEGER NOT NULL DEFAULT (unixepoch())
            );

            CREATE INDEX IF NOT EXISTS idx_memory_category ON memory(category);
            CREATE INDEX IF NOT EXISTS idx_memory_updated  ON memory(updated_at DESC);

            CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
                key,
                value,
                content=memory,
                content_rowid=id,
                tokenize='porter unicode61'
            );

            -- Triggers to keep memory_fts in sync with the memory table
            CREATE TRIGGER IF NOT EXISTS memory_ai AFTER INSERT ON memory BEGIN
                INSERT INTO memory_fts(rowid, key, value)
                VALUES (new.id, new.key, new.value);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_ad AFTER DELETE ON memory BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, key, value)
                VALUES ('delete', old.id, old.key, old.value);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_au AFTER UPDATE ON memory BEGIN
                INSERT INTO memory_fts(memory_fts, rowid, key, value)
                VALUES ('delete', old.id, old.key, old.value);
                INSERT INTO memory_fts(rowid, key, value)
                VALUES (new.id, new.key, new.value);
            END;"
        )?;

        debug!("RAG database opened at {}", db_path.display());
        Ok(Self { conn, db_path })
    }

    /// Total number of indexed chunks.
    pub fn chunk_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM code_chunks",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Number of unique files indexed.
    pub fn file_count(&self) -> Result<i64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT file_path) FROM code_chunks",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Get the mtime we last indexed for a file (0 if never indexed).
    pub fn file_mtime(&self, path: &str) -> Result<i64> {
        let result = self.conn.query_row(
            "SELECT MAX(mtime) FROM code_chunks WHERE file_path = ?1",
            [path],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        Ok(result.unwrap_or(0))
    }

    /// Delete all chunks for a given file (before re-indexing it).
    #[allow(dead_code)] // public API, used in tests, will be called by incremental re-index
    pub fn delete_file_chunks(&self, path: &str) -> Result<()> {
        self.conn.execute("DELETE FROM code_chunks WHERE file_path = ?1", [path])?;
        Ok(())
    }

    /// Delete all chunks (full re-index).
    pub fn clear(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM code_chunks;
             INSERT INTO chunks_fts(chunks_fts) VALUES ('rebuild');"
        )?;
        Ok(())
    }

    /// Database file size in bytes.
    pub fn db_size(&self) -> i64 {
        std::fs::metadata(&self.db_path)
            .map(|m| m.len() as i64)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_db() -> (TempDir, RagDb) {
        let tmp = TempDir::new().unwrap();
        let db = RagDb::open(tmp.path()).unwrap();
        (tmp, db)
    }

    #[test]
    fn test_open_creates_tables() {
        let (_tmp, db) = test_db();
        assert_eq!(db.chunk_count().unwrap(), 0);
        assert_eq!(db.file_count().unwrap(), 0);
        assert!(db.db_size() > 0);
    }

    #[test]
    fn test_insert_and_count() {
        let (_tmp, db) = test_db();
        db.conn.execute(
            "INSERT INTO code_chunks (file_path, symbol_name, symbol_kind, language, start_line, end_line, content, mtime)
             VALUES ('src/main.rs', 'main', 'function', 'rust', 1, 10, 'fn main() {}', 1000)",
            [],
        ).unwrap();
        assert_eq!(db.chunk_count().unwrap(), 1);
        assert_eq!(db.file_count().unwrap(), 1);
    }

    #[test]
    fn test_file_mtime() {
        let (_tmp, db) = test_db();
        assert_eq!(db.file_mtime("src/main.rs").unwrap(), 0);
        db.conn.execute(
            "INSERT INTO code_chunks (file_path, symbol_name, symbol_kind, language, start_line, end_line, content, mtime)
             VALUES ('src/main.rs', 'main', 'function', 'rust', 1, 10, 'fn main() {}', 42)",
            [],
        ).unwrap();
        assert_eq!(db.file_mtime("src/main.rs").unwrap(), 42);
    }

    #[test]
    fn test_delete_file_chunks() {
        let (_tmp, db) = test_db();
        db.conn.execute(
            "INSERT INTO code_chunks (file_path, symbol_name, symbol_kind, language, start_line, end_line, content, mtime)
             VALUES ('a.rs', 'foo', 'function', 'rust', 1, 5, 'fn foo() {}', 1)",
            [],
        ).unwrap();
        db.conn.execute(
            "INSERT INTO code_chunks (file_path, symbol_name, symbol_kind, language, start_line, end_line, content, mtime)
             VALUES ('b.rs', 'bar', 'function', 'rust', 1, 5, 'fn bar() {}', 1)",
            [],
        ).unwrap();
        assert_eq!(db.chunk_count().unwrap(), 2);
        db.delete_file_chunks("a.rs").unwrap();
        assert_eq!(db.chunk_count().unwrap(), 1);
        assert_eq!(db.file_count().unwrap(), 1);
    }

    #[test]
    fn test_clear() {
        let (_tmp, db) = test_db();
        for i in 0..5 {
            db.conn.execute(
                "INSERT INTO code_chunks (file_path, symbol_name, symbol_kind, language, start_line, end_line, content, mtime)
                 VALUES (?1, 'sym', 'function', 'rust', 1, 5, 'code', 1)",
                [format!("file{i}.rs")],
            ).unwrap();
        }
        assert_eq!(db.chunk_count().unwrap(), 5);
        db.clear().unwrap();
        assert_eq!(db.chunk_count().unwrap(), 0);
    }
}
