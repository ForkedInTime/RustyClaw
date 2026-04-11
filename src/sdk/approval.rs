//! Policy evaluation for tool approval.
//!
//! Evaluation order: deny > ask > auto_approve > allow.
//! Unlisted tools: ask (if interactive_approval) or deny (if not).

use crate::sdk::protocol::Policy;

/// What should happen when a tool is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Execute silently, no notification.
    Allow,
    /// Execute with a tool/started notification.
    AutoApprove,
    /// Send tool/approval_needed, block until host responds.
    Ask,
    /// Deny immediately.
    Deny,
}

/// Evaluates tool calls against the session policy.
pub struct PolicyEngine {
    policy: Policy,
    interactive_approval: bool,
}

impl PolicyEngine {
    pub fn new(policy: Policy, interactive_approval: bool) -> Self {
        Self {
            policy,
            interactive_approval,
        }
    }

    /// Evaluate a tool name against the policy.
    pub fn evaluate(&self, tool_name: &str) -> ApprovalDecision {
        // Deny takes highest priority
        if self.policy.deny.iter().any(|t| t == tool_name) {
            return ApprovalDecision::Deny;
        }
        // Ask is next
        if self.policy.ask.iter().any(|t| t == tool_name) {
            return ApprovalDecision::Ask;
        }
        // Auto-approve
        if self.policy.auto_approve.iter().any(|t| t == tool_name) {
            return ApprovalDecision::AutoApprove;
        }
        // Allow (silent)
        if self.policy.allow.iter().any(|t| t == tool_name) {
            return ApprovalDecision::Allow;
        }
        // Not in any list — depends on interactive_approval capability
        if self.interactive_approval {
            ApprovalDecision::Ask
        } else {
            ApprovalDecision::Deny
        }
    }

    /// Get the approval timeout in seconds.
    pub fn timeout_seconds(&self) -> u64 {
        self.policy.approval_timeout_seconds
    }
}
