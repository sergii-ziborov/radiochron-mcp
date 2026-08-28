//! The command line: a second front end over the MCP tool handlers.
//!
//! Every command routes through [`crate::mcp_server::run_once`], so a CLI
//! result and an MCP result are produced by the same code with the same bounds
//! and the same error text. What lives here is the human spelling — the flags,
//! the routing and the rendering — and nothing that decides an answer.
//!
//! Invoked with no arguments the binary still serves MCP over stdio, because
//! every installed client is configured to run it exactly that way. A person
//! who types `radiochron` at a prompt gets help instead: a terminal on stdin
//! separates the two without a flag anyone has to add to an existing config.

mod args;
mod commands;
mod render;

use std::io::IsTerminal;

use args::Flags;

use crate::chronicle::ChronicleService;
use crate::mcp_server;

const HELP: &str = "\
radiochron — Wi-Fi and Bluetooth LE diagnostics

USAGE
  radiochron <command> [flags]
  radiochron                        serve MCP over stdio (when stdin is a pipe)

WI-FI
  status                            association state of every adapter
  scan                              ask the driver to look again, then list what is there
  networks [--refresh] [--detail summary|full]
                                    list networks from the scan cache
  analyze [--refresh]               findings about the surrounding environment
  report                            the full diagnostic report, as Markdown
  history [--within S] [--max N] [--events]
                                    connection history from the OS log (Windows only)
  sample [--duration S] [--interval MS] [--interface GUID]
                                    track signal and rate over a window

CONNECTIVITY
  connectivity [--dns NAME] [--tcp HOST:PORT] [--internet URL]
               [--captive-portal URL] [--captive-status CODE] [--tls HOST:PORT]
               [--quality HOST:PORT] [--attempts N] [--timeout MS]
                                    where the path to the Internet breaks
  incident [--refresh] [--ble] [--ble-ms MS] [--sensor-id ID] [--zone Z]
           [--session ID] [--moving] [--within S] [--max-events N]
           [--chronicle-max N] and every connectivity flag above
                                    one composite answer for \"the Wi-Fi is broken\"

BLUETOOTH LE
  ble scan [--duration MS] [--sensor-id ID] [--zone Z] [--session ID] [--moving]
                                    native BLE scan with evidence-based detectors
  ble histories                     identities observed during this run
  ble identify --advertisement '<json>'
                                    protocol fingerprint for one advertisement

CHRONICLE
  chronicle recent [--max N]        recent entries from the change journal
  chronicle record [--interval S] [--threshold DB]
                                    record in the foreground until you stop it
  chronicle path                    where the journal is stored

OTHER
  mcp                               serve MCP over stdio explicitly
  tool <name> [--args '<json>']     call any MCP tool directly
  --json                            print the raw JSON result, for any command
  --version  --build-info  --help

A Wi-Fi scan lists neighbouring SSIDs and BSSIDs, and a BSSID resolves to a street
address through public geolocation databases. Treat the output as location data.
";

/// Run the binary for `argv`, the arguments after the program name.
pub fn run(argv: &[String]) -> anyhow::Result<()> {
    let mut tokens: Vec<String> = argv.to_vec();
    let as_json = take(&mut tokens, "--json");

    let Some(command) = tokens.first().cloned() else {
        if as_json {
            anyhow::bail!("--json needs a command; run radiochron --help for the list");
        }
        return if std::io::stdin().is_terminal() {
            print!("{HELP}");
            Ok(())
        } else {
            mcp_server::serve_stdio()
        };
    };
    let rest = &tokens[1..];

    match command.as_str() {
        "--version" | "-V" => {
            println!("radiochron {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--build-info" => {
            println!(
                "{{\"name\":\"radiochron\",\"version\":\"{}\",\"git_sha\":\"{}\",\"platform\":\"{}\",\"arch\":\"{}\"}}",
                env!("CARGO_PKG_VERSION"),
                env!("RADIOCHRON_GIT_SHA"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            Ok(())
        }
        "--help" | "-h" | "help" => {
            print!("{HELP}");
            Ok(())
        }
        "mcp" => mcp_server::serve_stdio(),
        "report" => report(as_json),
        "tool" => {
            let (name, arguments) = commands::raw_tool_call(rest)?;
            let value = mcp_server::run_once(&name, &arguments)?;
            render::emit(&name, &value, true)
        }
        "chronicle" => chronicle(rest, as_json),
        "ble" => {
            let sub = subcommand("ble", rest, &["scan", "histories", "identify"])?;
            call(&sub, &rest[1..], as_json)
        }
        other => call(other, rest, as_json),
    }
}

fn call(command: &str, tokens: &[String], as_json: bool) -> anyhow::Result<()> {
    let (tool, arguments) = commands::tool_call(command, tokens)?;
    if tool == "wifi_sample" {
        // Sampling holds the terminal for as long as it was asked to. Saying so
        // beats a silent process that looks hung; it goes to stderr so `--json`
        // output stays pipeable.
        eprintln!("Sampling — this runs for the requested window, then reports.");
    }
    let value = mcp_server::run_once(tool, &arguments)?;
    render::emit(command, &value, as_json)
}

fn chronicle(tokens: &[String], as_json: bool) -> anyhow::Result<()> {
    let sub = subcommand("chronicle", tokens, &["recent", "record", "path"])?;
    let rest = &tokens[1..];
    match sub.as_str() {
        "chronicle path" => {
            Flags::parse(rest, &[], &[])?;
            let status = ChronicleService::new().status();
            println!(
                "{}",
                status
                    .get("path")
                    .and_then(blazingly_json::Value::as_str)
                    .unwrap_or("unknown")
            );
            Ok(())
        }
        "chronicle record" => record(rest),
        _ => call(&sub, rest, as_json),
    }
}

/// Record in the foreground until the operator stops it.
///
/// The MCP `chronicle_start` tool hands a recorder to a session that outlives
/// the call. A one-shot process has no such session, so the CLI keeps the
/// recorder in front of the operator and stops it on demand — which also flushes
/// and reports, rather than leaving the journal to a killed process.
fn record(tokens: &[String]) -> anyhow::Result<()> {
    let flags = Flags::parse(tokens, &["interval", "threshold"], &[])?;
    let (interval, threshold) =
        mcp_server::recording_options(flags.integer("interval")?, flags.integer("threshold")?)?;

    let service = ChronicleService::new();
    let started = service.start(interval, threshold)?;
    let path = started
        .get("path")
        .and_then(blazingly_json::Value::as_str)
        .unwrap_or("the chronicle");
    eprintln!(
        "Recording every {}s to {path}\nPress Enter to stop (Ctrl-C also works).",
        interval.as_secs()
    );

    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    let stopped = service.stop()?;
    println!("{}", blazingly_json::to_string_pretty(&stopped)?);
    Ok(())
}

fn report(as_json: bool) -> anyhow::Result<()> {
    let body = if as_json {
        mcp_server::report_json()?
    } else {
        mcp_server::report_markdown()?
    };
    println!("{body}");
    Ok(())
}

/// Join a command with its subcommand, or explain what was expected.
fn subcommand(parent: &str, tokens: &[String], valid: &[&str]) -> anyhow::Result<String> {
    let Some(sub) = tokens.first() else {
        anyhow::bail!("{parent} needs a subcommand: {}", valid.join(", "));
    };
    if !valid.contains(&sub.as_str()) {
        anyhow::bail!(
            "unknown {parent} subcommand: {sub}\nExpected one of: {}",
            valid.join(", ")
        );
    }
    Ok(format!("{parent} {sub}"))
}

/// Remove `flag` wherever it appears, reporting whether it was there.
fn take(tokens: &mut Vec<String>, flag: &str) -> bool {
    let before = tokens.len();
    tokens.retain(|token| token != flag);
    tokens.len() != before
}

#[cfg(test)]
mod tests {
    use super::{subcommand, take, HELP};

    fn tokens(input: &[&str]) -> Vec<String> {
        input.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn json_is_accepted_in_any_position() {
        let mut left = tokens(&["--json", "networks", "--refresh"]);
        assert!(take(&mut left, "--json"));
        assert_eq!(left, tokens(&["networks", "--refresh"]));

        let mut right = tokens(&["networks", "--refresh", "--json"]);
        assert!(take(&mut right, "--json"));
        assert_eq!(right, tokens(&["networks", "--refresh"]));

        let mut absent = tokens(&["networks"]);
        assert!(!take(&mut absent, "--json"));
    }

    #[test]
    fn subcommands_are_joined_or_explained() {
        assert_eq!(
            subcommand("ble", &tokens(&["scan"]), &["scan", "histories"]).unwrap(),
            "ble scan"
        );
        let missing = subcommand("ble", &[], &["scan", "histories"])
            .expect_err("must ask for a subcommand")
            .to_string();
        assert!(missing.contains("scan, histories"));

        let wrong = subcommand("ble", &tokens(&["sniff"]), &["scan"])
            .expect_err("must reject")
            .to_string();
        assert!(wrong.contains("sniff"));
    }

    #[test]
    fn help_documents_every_command_the_router_accepts() {
        for command in [
            "status",
            "scan",
            "networks",
            "analyze",
            "report",
            "history",
            "sample",
            "connectivity",
            "incident",
            "ble scan",
            "ble histories",
            "ble identify",
            "chronicle recent",
            "chronicle record",
            "chronicle path",
            "mcp",
            "tool",
        ] {
            assert!(HELP.contains(command), "{command} is missing from --help");
        }
    }

    #[test]
    fn help_keeps_the_location_warning() {
        assert!(HELP.contains("location data"));
    }
}
