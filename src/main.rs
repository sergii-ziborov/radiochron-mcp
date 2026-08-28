//! RadioChron: Wi-Fi and Bluetooth LE diagnostics, as a command line and as an
//! MCP server.
//!
//! A thin shell over the `radiochron` library: this crate owns the protocol, the
//! command line and the report rendering, and nothing else. All collection and
//! analysis lives in the library, so an IoT agent can use the same engine
//! without dragging JSON-RPC along.
//!
//! The two front ends are not two implementations. Commands route through the
//! same tool handlers the protocol calls, so a `radiochron analyze` answer and a
//! `wifi_analyze` tool result come from one code path with one set of bounds.
//!
//! Registering it with an MCP client (Claude Code, Claude Desktop, Codex, …)
//! still means pointing the client at this binary with no arguments. On the
//! stdio transport stdout carries JSON-RPC frames and nothing else, so all
//! diagnostics must go to stderr.

mod ble;
mod ble_scan;
mod chronicle;
mod cli;
mod mcp_server;
mod report;

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    cli::run(&arguments)
}
