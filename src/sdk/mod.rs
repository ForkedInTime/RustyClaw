//! RustyClaw SDK — headless NDJSON server for embedding.

pub mod protocol;
pub mod transport;
pub mod approval;
pub mod session;

pub use protocol::*;

use crate::config::Config;
use anyhow::Result;

/// Headless SDK server — reads NDJSON requests, writes NDJSON responses.
pub struct SdkServer;

impl SdkServer {
    /// Run the server loop until the transport closes (EOF on stdin).
    pub async fn run<T: transport::Transport>(
        _config: Config,
        mut transport: T,
    ) -> Result<()> {
        eprintln!("[sdk] RustyClaw SDK server starting...");
        loop {
            match transport.read_request().await? {
                Some(req) => {
                    // TODO: dispatch requests to handlers
                    let id = match &req {
                        SdkRequest::HealthCheck { id } => id.clone(),
                        SdkRequest::SessionStart { id, .. } => id.clone(),
                        SdkRequest::SessionResume { id, .. } => id.clone(),
                        SdkRequest::SessionList { id, .. } => id.clone(),
                        SdkRequest::SessionExport { id, .. } => id.clone(),
                        SdkRequest::TurnStart { id, .. } => id.clone(),
                        SdkRequest::TurnInterrupt { id, .. } => id.clone(),
                        SdkRequest::ToolApprove { id, .. } => id.clone(),
                        SdkRequest::ToolDeny { id, .. } => id.clone(),
                        SdkRequest::RagSearch { id, .. } => id.clone(),
                        SdkRequest::CostReport { id, .. } => id.clone(),
                    };
                    // For now, respond with an error for unimplemented requests
                    // Health check is the only implemented handler
                    let response = match req {
                        SdkRequest::HealthCheck { id } => SdkResponse::HealthCheck {
                            id,
                            status: "ok".into(),
                            version: env!("CARGO_PKG_VERSION").into(),
                            active_sessions: 0,
                            uptime_seconds: 0,
                        },
                        _ => SdkResponse::Error {
                            id,
                            code: "not_implemented".into(),
                            message: "Request type not yet implemented".into(),
                        },
                    };
                    transport.send_response(response).await?;
                }
                None => {
                    eprintln!("[sdk] Transport closed, shutting down.");
                    break;
                }
            }
        }
        Ok(())
    }
}
