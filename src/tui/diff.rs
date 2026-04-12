//! Unified diff parsing and hunk-level state management for the diff review UI.
//!
//! This module is parser-only for now. A follow-up task wires it into the TUI
//! (overlay render + key dispatch + /diff command integration).

#[derive(Debug, Clone, PartialEq)]
pub enum HunkState {
    Pending,
    Accepted,
    Rejected,
}

impl HunkState {
    pub fn toggle(self) -> Self {
        match self {
            Self::Pending => Self::Accepted,
            Self::Accepted => Self::Rejected,
            Self::Rejected => Self::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub header: String, // @@ -1,3 +1,4 @@
    pub lines: Vec<DiffLine>,
    pub state: HunkState,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineKind {
    Context, // unchanged line (space prefix)
    Added,   // + line
    Removed, // - line
    Header,  // @@ header or file header
}

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

/// Parse a unified diff string (output of `git diff`) into structured FileDiffs.
///
/// Handles multi-file diffs. File path is taken from the `b/<path>` side of
/// the `diff --git` header (the post-image path), matching how git describes
/// the target tree. Lines that aren't part of a hunk (index / ---  / +++) are
/// skipped.
pub fn parse_unified_diff(diff: &str) -> Vec<FileDiff> {
    let mut files = Vec::new();
    let mut current_path = String::new();
    let mut current_hunks: Vec<DiffHunk> = Vec::new();
    let mut current_lines: Vec<DiffLine> = Vec::new();
    let mut current_header = String::new();
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for line in diff.lines() {
        if line.starts_with("diff --git") {
            // Flush any in-progress hunk, then the in-progress file.
            if !current_path.is_empty() {
                if !current_lines.is_empty() {
                    current_hunks.push(DiffHunk {
                        header: current_header.clone(),
                        lines: std::mem::take(&mut current_lines),
                        state: HunkState::Pending,
                    });
                }
                files.push(FileDiff {
                    path: std::mem::take(&mut current_path),
                    hunks: std::mem::take(&mut current_hunks),
                    additions,
                    deletions,
                });
                additions = 0;
                deletions = 0;
            }
            // `diff --git a/path b/path` — take the b/ side.
            if let Some(b_part) = line.split(" b/").nth(1) {
                current_path = b_part.to_string();
            }
        } else if line.starts_with("@@") {
            // New hunk — flush the previous one.
            if !current_lines.is_empty() {
                current_hunks.push(DiffHunk {
                    header: current_header.clone(),
                    lines: std::mem::take(&mut current_lines),
                    state: HunkState::Pending,
                });
            }
            current_header = line.to_string();
        } else if line.starts_with("+++") || line.starts_with("---") || line.starts_with("index ") {
            // Skip file/index headers — they're not part of any hunk body.
        } else if let Some(rest) = line.strip_prefix('+') {
            additions += 1;
            current_lines.push(DiffLine {
                kind: DiffLineKind::Added,
                content: rest.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix('-') {
            deletions += 1;
            current_lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                content: rest.to_string(),
            });
        } else if line.starts_with(' ') || line.is_empty() {
            let content = if line.is_empty() {
                String::new()
            } else {
                line[1..].to_string()
            };
            current_lines.push(DiffLine {
                kind: DiffLineKind::Context,
                content,
            });
        }
        // Any other prefix (e.g. "\\ No newline at end of file") is ignored.
    }

    // Flush trailing state.
    if !current_path.is_empty() {
        if !current_lines.is_empty() {
            current_hunks.push(DiffHunk {
                header: current_header,
                lines: current_lines,
                state: HunkState::Pending,
            });
        }
        files.push(FileDiff {
            path: current_path,
            hunks: current_hunks,
            additions,
            deletions,
        });
    }

    files
}

/// Scrollable review state — cursor over files and their hunks.
#[allow(dead_code)] // wired in Task 12
pub struct DiffReviewState {
    pub files: Vec<FileDiff>,
    pub current_file: usize,
    pub current_hunk: usize,
    pub scroll: usize,
}

#[allow(dead_code)] // wired in Task 12
impl DiffReviewState {
    pub fn new(files: Vec<FileDiff>) -> Self {
        Self {
            files,
            current_file: 0,
            current_hunk: 0,
            scroll: 0,
        }
    }

    pub fn total_hunks(&self) -> usize {
        self.files.iter().map(|f| f.hunks.len()).sum()
    }

    pub fn toggle_current(&mut self) {
        if let Some(file) = self.files.get_mut(self.current_file)
            && let Some(hunk) = file.hunks.get_mut(self.current_hunk)
        {
            hunk.state = hunk.state.clone().toggle();
        }
    }

    pub fn next_hunk(&mut self) {
        if let Some(file) = self.files.get(self.current_file) {
            if self.current_hunk + 1 < file.hunks.len() {
                self.current_hunk += 1;
            } else if self.current_file + 1 < self.files.len() {
                self.current_file += 1;
                self.current_hunk = 0;
            }
        }
    }

    pub fn prev_hunk(&mut self) {
        if self.current_hunk > 0 {
            self.current_hunk -= 1;
        } else if self.current_file > 0 {
            self.current_file -= 1;
            self.current_hunk = self.files[self.current_file].hunks.len().saturating_sub(1);
        }
    }

    pub fn accept_all(&mut self) {
        for file in &mut self.files {
            for hunk in &mut file.hunks {
                hunk.state = HunkState::Accepted;
            }
        }
    }

    pub fn reject_all(&mut self) {
        for file in &mut self.files {
            for hunk in &mut file.hunks {
                hunk.state = HunkState::Rejected;
            }
        }
    }
}
