//! Browser automation via Chrome DevTools Protocol.
//!
//! Architecture: CdpClient (WebSocket) -> BrowserSession (lifecycle) -> actions/snapshot/extraction.
pub mod cdp;
