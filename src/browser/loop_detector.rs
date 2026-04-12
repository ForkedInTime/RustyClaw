//! Action fingerprinting and stagnation detection.
//!
//! Tracks a rolling window of (action_hash, page_state_hash) pairs.
//! Detects when the agent repeats the same action on the same page state.
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

const WINDOW_SIZE: usize = 10;
const REPEAT_THRESHOLD: usize = 3;

/// Nudge messages, escalating in severity.
const NUDGES: [&str; 3] = [
    "You seem to be repeating the same action with no effect. Try a different approach — \
     perhaps a different element, a different selector, or navigate to a different section.",
    "This action has failed multiple times on the same page state. Consider: \
     (1) the element may be disabled or overlaid, (2) you may need to scroll first, \
     (3) try using JavaScript evaluation as a fallback, (4) the page may require authentication.",
    "Stopping — the browser agent has repeated the same action 3 times with no progress. \
     The page may be stuck, require a CAPTCHA, or the target element may not be interactable.",
];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ActionFingerprint {
    action_hash: String,
    page_hash: String,
}

pub struct LoopDetector {
    window: VecDeque<ActionFingerprint>,
    nudge_level: usize,
}

impl LoopDetector {
    pub fn new() -> Self {
        Self {
            window: VecDeque::with_capacity(WINDOW_SIZE),
            nudge_level: 0,
        }
    }

    /// Record an action + current page state.
    pub fn record_action(&mut self, action_type: &str, target: &str, page_text: &str) {
        let fp = ActionFingerprint {
            action_hash: fingerprint_action(action_type, target, ""),
            page_hash: hash_string(page_text),
        };
        self.window.push_back(fp);
        if self.window.len() > WINDOW_SIZE {
            self.window.pop_front();
        }
    }

    /// Check for stagnation. Returns Some(nudge_message) if stuck.
    ///
    /// Fires when the most recent `REPEAT_THRESHOLD` actions are all the same
    /// fingerprint (same action + target + page state). A single differing
    /// action in between breaks the run — we only care about tight loops.
    ///
    /// Nudge escalation is intentionally sticky across this call: callers
    /// should invoke [`reset`](Self::reset) on navigation / intentional
    /// context switches to start fresh at the gentlest nudge.
    pub fn check_stagnation(&mut self) -> Option<String> {
        if self.window.len() < REPEAT_THRESHOLD {
            return None;
        }

        let last = self.window.back()?.clone();
        let all_same = self
            .window
            .iter()
            .rev()
            .take(REPEAT_THRESHOLD)
            .all(|fp| *fp == last);

        if all_same {
            let nudge = NUDGES[self.nudge_level.min(NUDGES.len() - 1)].to_string();
            if self.nudge_level < NUDGES.len() - 1 {
                self.nudge_level += 1;
            }
            Some(nudge)
        } else {
            None
        }
    }

    /// Reset the detector (e.g. after navigating to a new page).
    pub fn reset(&mut self) {
        self.window.clear();
        self.nudge_level = 0;
    }
}

impl Default for LoopDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash an action (type + target) into a deterministic fingerprint.
///
/// `_extra` is reserved for future use (e.g., hashing input values for
/// `fill` actions to distinguish retries with different content). For now
/// only action type and target contribute to the fingerprint.
pub fn fingerprint_action(action_type: &str, target: &str, _extra: &str) -> String {
    hash_string(&format!("{action_type}:{target}"))
}

fn hash_string(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    format!("{:x}", hasher.finalize())
}
