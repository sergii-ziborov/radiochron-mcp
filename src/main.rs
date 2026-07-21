//! RadioChron MCP server.
//!
//! A thin transport shell over the `radiochron` library: this crate owns the
//! protocol and the report rendering, and nothing else. All collection and
//! analysis lives in the library, so an IoT agent or a CLI can use the same
//! engine without dragging JSON-RPC along.
//!
//! Speaks the Model Context Protocol over stdio. Register it with an MCP client
//! (Claude Code, Claude Desktop, Codex, …) by pointing the client at this
//! binary; no arguments are required.
//!
//! On the stdio transport stdout carries JSON-RPC frames and nothing else, so
//! all diagnostics must go to stderr.

#[cfg(windows)]
mod chronicle;
#[cfg(windows)]
mod mcp_server;
#[cfg(windows)]
mod report;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--version" | "-V") => {
            println!("radiochron {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--build-info") => {
            println!(
                "{{\"name\":\"radiochron\",\"version\":\"{}\",\"git_sha\":\"{}\"}}",
                env!("CARGO_PKG_VERSION"),
                env!("RADIOCHRON_GIT_SHA")
            );
            return Ok(());
        }
        Some("--help" | "-h") => {
            println!(
                "radiochron [--version|--build-info]\n\nWithout arguments, serves MCP over stdio."
            );
            return Ok(());
        }
        Some(other) => anyhow::bail!("unknown argument: {other}"),
        None => {}
    }
    mcp_server::serve_stdio()
}

#[cfg(not(windows))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("RadioChron requires Windows (it talks to wlanapi.dll).")
}
