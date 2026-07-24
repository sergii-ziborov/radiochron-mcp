pub(crate) mod catalog;
mod protocol;
mod resources;
pub(crate) mod schema;
mod tools;
pub(crate) mod transport;

#[cfg(test)]
mod tests;

use std::time::Duration;

pub use transport::serve_stdio;

pub(super) const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
pub(super) const LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";
pub(super) const SUPPORTED_PROTOCOL_VERSIONS: [&str; 2] =
    [LATEST_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION];
pub(super) const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const MAX_HISTORY_WINDOW_S: u64 = 365 * 24 * 60 * 60;

pub(super) const PARSE_ERROR: i64 = -32700;
pub(super) const INVALID_REQUEST: i64 = -32600;
pub(super) const METHOD_NOT_FOUND: i64 = -32601;
pub(super) const INVALID_PARAMS: i64 = -32602;
pub(super) const INTERNAL_ERROR: i64 = -32603;
pub(super) const RESOURCE_NOT_FOUND: i64 = -32002;
