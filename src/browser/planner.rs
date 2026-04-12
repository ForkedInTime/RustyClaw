//! Autonomous browser agent loop.
//!
//! For multi-step browser tasks (e.g. `/browse find cheapest flight to Tokyo`),
//! this module runs a plan-act-evaluate loop with the LLM, using the loop
//! detector to prevent stagnation. For now only the *interface contract* is
//! here — the actual loop requires access to the API client (lives in
//! `run.rs`), so the wiring happens in a follow-up task.

#![allow(dead_code)] // wired in a follow-up task; keep the shape stable

use super::BrowserSession;
use super::loop_detector::LoopDetector;
use anyhow::Result;

/// Maximum steps before the planner gives up.
pub const MAX_STEPS: usize = 25;

/// Result of a single planning step.
#[derive(Debug)]
pub struct PlanStep {
    pub action: String,
    pub target: String,
    pub value: Option<String>,
    pub evaluation: String,
    pub plan_update: Option<String>,
    pub goal_achieved: bool,
}

/// Run the autonomous browser agent loop.
/// Returns the final result text.
///
/// The loop is: ask the LLM for a structured `PlanStep`, execute it against
/// `session`, record the action in `loop_detector`, check for stagnation, then
/// feed the new page state back to the LLM. Bails when `goal_achieved` is
/// `true`, when `loop_detector.check_stagnation()` returns the level-3 "stop"
/// nudge, or when `MAX_STEPS` is exceeded.
///
/// Stub for now — the actual implementation needs the API client from
/// `run.rs`. Kept here so the interface is stable.
pub async fn run_autonomous(
    _session: &mut BrowserSession,
    _goal: &str,
    _loop_detector: &mut LoopDetector,
) -> Result<String> {
    Ok("Autonomous browser mode is not yet wired to the API client. \
        Use the browser_* tools directly."
        .into())
}
