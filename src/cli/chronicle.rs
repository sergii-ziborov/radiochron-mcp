//! The chronicle commands.
//!
//! `recent` is an ordinary tool call. The other two exist only here: the MCP
//! `chronicle_start`/`chronicle_stop` pair belongs to a session that outlives a
//! request, and a one-shot process has no such session to hand a recorder to.

use std::io::IsTerminal;

use super::args::Flags;
use crate::chronicle::ChronicleService;
use crate::mcp_server;

pub(super) fn dispatch(tokens: &[String], as_json: bool) -> anyhow::Result<()> {
    let sub = super::subcommand("chronicle", tokens, &["recent", "record", "path"])?;
    let rest = &tokens[1..];
    match sub.as_str() {
        "chronicle path" => path(rest),
        "chronicle record" => record(rest),
        _ => super::call(&sub, rest, as_json),
    }
}

fn path(tokens: &[String]) -> anyhow::Result<()> {
    Flags::parse(tokens, &[], &[])?;
    println!("{}", text(&ChronicleService::new().status(), "unknown"));
    Ok(())
}

/// Record in the foreground until whoever started it stops it.
///
/// How it waits depends on who that is. At a terminal, Enter stops the recorder
/// through [`ChronicleService::stop`], which joins the worker and reports what
/// it wrote. Under a supervisor there is nobody to press Enter and stdin is
/// usually closed, so waiting on it would stop the recorder immediately; there
/// the process parks until a signal. Nothing is lost either way — the sink
/// writes each entry as it happens rather than at shutdown.
fn record(tokens: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(tokens, &["interval", "threshold"], &[])?;
    let (interval, threshold) =
        mcp_server::recording_options(flags.integer("interval")?, flags.integer("threshold")?)?;

    let service = ChronicleService::new();
    let started = service.start(interval, threshold)?;
    let destination = text(&started, "the chronicle");
    let every = interval.as_secs();

    if !std::io::stdin().is_terminal() {
        eprintln!("Recording every {every}s to {destination}. Stop with SIGINT or SIGTERM.");
        loop {
            std::thread::park();
        }
    }

    eprintln!(
        "Recording every {every}s to {destination}\nPress Enter to stop (Ctrl-C also works)."
    );
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;

    let stopped = service.stop()?;
    println!("{}", blazingly_json::to_string_pretty(&stopped)?);
    Ok(())
}

/// The `path` a chronicle reply carries, or `fallback` when it carries none.
fn text(value: &blazingly_json::Value, fallback: &str) -> String {
    value
        .get("path")
        .and_then(blazingly_json::Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}
