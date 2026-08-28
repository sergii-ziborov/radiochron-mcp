//! Command line spelling of the MCP tool arguments.
//!
//! Each command translates flags into the argument object its tool already
//! validates. Nothing here checks a range: `--duration 9999` is rejected by the
//! same bound the MCP client would hit, with the same message, because the
//! handler is the same one. Flag names are the human spelling (`--within`) and
//! the tool keeps the protocol spelling (`history_within_seconds`).

mod builder;

use blazingly_json::Value;

use super::args::Flags;
use builder::Builder;

/// Flags shared by `connectivity` and `incident`.
const CONNECTIVITY_VALUES: &[&str] = &[
    "dns",
    "tcp",
    "internet",
    "captive-portal",
    "captive-status",
    "tls",
    "quality",
    "attempts",
    "timeout",
];

/// Resolve a command and its flags into the tool call that serves it.
pub(super) fn tool_call(command: &str, tokens: &[String]) -> anyhow::Result<(&'static str, Value)> {
    let mut fields = Builder::default();
    let tool = match command {
        "status" => {
            Flags::parse(tokens, &[], &[])?;
            "wifi_status"
        }
        "scan" | "networks" => {
            let flags = Flags::parse(tokens, &["detail"], &["refresh"])?;
            // `scan` exists so that the common intent — "look again, now" — is
            // one word instead of a flag someone has to remember.
            fields.set(
                "refresh_scan",
                Value::Bool(command == "scan" || flags.on("refresh")),
            );
            fields.text("detail", flags.text("detail"));
            "wifi_networks"
        }
        "analyze" => {
            let flags = Flags::parse(tokens, &[], &["refresh"])?;
            fields.flag("refresh_scan", flags.on("refresh"));
            "wifi_analyze"
        }
        "history" => {
            let flags = Flags::parse(tokens, &["within", "max"], &["events"])?;
            fields.integer("within_seconds", flags.integer("within")?);
            fields.integer("max_events", flags.integer("max")?);
            fields.flag("include_events", flags.on("events"));
            "wifi_history"
        }
        "sample" => {
            let flags = Flags::parse(tokens, &["duration", "interval", "interface"], &[])?;
            fields.integer("duration_seconds", flags.integer("duration")?);
            fields.integer("interval_ms", flags.integer("interval")?);
            fields.text("interface_guid", flags.text("interface"));
            "wifi_sample"
        }
        "connectivity" => {
            let flags = Flags::parse(tokens, CONNECTIVITY_VALUES, &[])?;
            connectivity(&mut fields, &flags)?;
            "connectivity_diagnose"
        }
        "incident" => {
            let values: Vec<&str> = CONNECTIVITY_VALUES
                .iter()
                .copied()
                .chain([
                    "ble-ms",
                    "sensor-id",
                    "zone",
                    "session",
                    "within",
                    "max-events",
                    "chronicle-max",
                ])
                .collect();
            let flags = Flags::parse(tokens, &values, &["refresh", "ble", "moving"])?;
            connectivity(&mut fields, &flags)?;
            fields.flag("refresh_wifi", flags.on("refresh"));
            fields.flag("include_ble", flags.on("ble"));
            fields.flag("sensor_is_moving", flags.on("moving"));
            fields.integer("ble_scan_ms", flags.integer("ble-ms")?);
            fields.text("sensor_id", flags.text("sensor-id"));
            fields.text("zone", flags.text("zone"));
            fields.text("movement_session", flags.text("session"));
            fields.integer("history_within_seconds", flags.integer("within")?);
            fields.integer("history_max_events", flags.integer("max-events")?);
            fields.integer("chronicle_max_entries", flags.integer("chronicle-max")?);
            "diagnose_incident"
        }
        "ble scan" => {
            let flags = Flags::parse(
                tokens,
                &["duration", "sensor-id", "zone", "session"],
                &["moving"],
            )?;
            fields.integer("duration_ms", flags.integer("duration")?);
            fields.text("sensor_id", flags.text("sensor-id"));
            fields.text("zone", flags.text("zone"));
            fields.text("movement_session", flags.text("session"));
            fields.flag("sensor_is_moving", flags.on("moving"));
            "ble_scan"
        }
        "ble histories" => {
            Flags::parse(tokens, &[], &[])?;
            "ble_histories"
        }
        "ble identify" => {
            let flags = Flags::parse(tokens, &["advertisement"], &[])?;
            let raw = flags.text("advertisement").ok_or_else(|| {
                anyhow::anyhow!(
                    "ble identify needs --advertisement '<json>'; see radiochron --help"
                )
            })?;
            fields.set("advertisement", parse_json("--advertisement", raw)?);
            "ble_identify"
        }
        "chronicle recent" => {
            let flags = Flags::parse(tokens, &["max"], &[])?;
            fields.integer("max_entries", flags.integer("max")?);
            "chronicle_recent"
        }
        other => anyhow::bail!("unknown command: {other}\nRun radiochron --help for the list."),
    };
    Ok((tool, fields.build()))
}

/// `radiochron tool <name> [--args '<json>']`, the escape hatch.
///
/// Commands cover what a person types; this covers the rest of the catalogue
/// without inventing a flag for every argument of every tool, and it is how a
/// script reaches a tool the CLI has no friendly spelling for.
pub(super) fn raw_tool_call(tokens: &[String]) -> anyhow::Result<(String, Value)> {
    let Some(name) = tokens.first() else {
        anyhow::bail!(
            "tool needs a name, for example: radiochron tool wifi_scan\nAvailable: {}",
            crate::mcp_server::TOOL_NAMES.join(", ")
        );
    };
    let flags = Flags::parse(&tokens[1..], &["args"], &[])?;
    let arguments = match flags.text("args") {
        None => Builder::default().build(),
        Some(raw) => parse_json("--args", raw)?,
    };
    if !arguments.is_object() {
        anyhow::bail!("--args must be a JSON object, for example --args '{{\"max_entries\":5}}'");
    }
    Ok((name.clone(), arguments))
}

fn connectivity(fields: &mut Builder, flags: &Flags) -> anyhow::Result<()> {
    fields.text("dns_name", flags.text("dns"));
    fields.text("tcp_target", flags.text("tcp"));
    fields.text("internet_target", flags.text("internet"));
    fields.text("captive_portal_url", flags.text("captive-portal"));
    fields.integer(
        "captive_portal_expected_status",
        flags.integer("captive-status")?,
    );
    fields.text("tls_target", flags.text("tls"));
    fields.text("quality_target", flags.text("quality"));
    fields.integer("quality_attempts", flags.integer("attempts")?);
    fields.integer("timeout_ms", flags.integer("timeout")?);
    Ok(())
}

fn parse_json(flag: &str, raw: &str) -> anyhow::Result<Value> {
    blazingly_json::from_str(raw)
        .map_err(|error| anyhow::anyhow!("{flag} must be valid JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{raw_tool_call, tool_call};

    fn tokens(input: &[&str]) -> Vec<String> {
        input.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn scan_refreshes_and_networks_does_not() {
        let (tool, arguments) = tool_call("scan", &[]).unwrap();
        assert_eq!(tool, "wifi_networks");
        assert_eq!(arguments["refresh_scan"], true);

        let (_, arguments) = tool_call("networks", &[]).unwrap();
        assert_eq!(arguments["refresh_scan"], false);

        let (_, arguments) = tool_call("networks", &tokens(&["--refresh"])).unwrap();
        assert_eq!(arguments["refresh_scan"], true);
    }

    #[test]
    fn absent_flags_are_absent_not_null() {
        let (_, arguments) = tool_call("sample", &tokens(&["--duration", "30"])).unwrap();
        assert_eq!(arguments["duration_seconds"], 30);
        assert!(arguments.get("interval_ms").is_none());
        assert!(arguments.get("interface_guid").is_none());
    }

    #[test]
    fn human_flags_map_to_protocol_argument_names() {
        let (tool, arguments) = tool_call(
            "history",
            &tokens(&["--within", "600", "--max", "10", "--events"]),
        )
        .unwrap();
        assert_eq!(tool, "wifi_history");
        assert_eq!(arguments["within_seconds"], 600);
        assert_eq!(arguments["max_events"], 10);
        assert_eq!(arguments["include_events"], true);
    }

    #[test]
    fn connectivity_flags_reach_their_targets() {
        let (tool, arguments) = tool_call(
            "connectivity",
            &tokens(&[
                "--dns",
                "example.com",
                "--timeout",
                "1500",
                "--attempts",
                "3",
            ]),
        )
        .unwrap();
        assert_eq!(tool, "connectivity_diagnose");
        assert_eq!(arguments["dns_name"], "example.com");
        assert_eq!(arguments["timeout_ms"], 1500);
        assert_eq!(arguments["quality_attempts"], 3);
    }

    #[test]
    fn incident_accepts_both_its_own_and_the_connectivity_flags() {
        let (tool, arguments) = tool_call(
            "incident",
            &tokens(&["--ble", "--dns", "example.com", "--zone", "lab"]),
        )
        .unwrap();
        assert_eq!(tool, "diagnose_incident");
        assert_eq!(arguments["include_ble"], true);
        assert_eq!(arguments["dns_name"], "example.com");
        assert_eq!(arguments["zone"], "lab");
    }

    #[test]
    fn unknown_commands_and_flags_are_refused() {
        assert!(tool_call("teleport", &[]).is_err());
        assert!(tool_call("status", &tokens(&["--refresh"])).is_err());
        assert!(tool_call("ble identify", &[]).is_err());
        assert!(tool_call("ble identify", &tokens(&["--advertisement", "{oops"])).is_err());
    }

    #[test]
    fn raw_tool_defaults_to_an_empty_object() {
        let (name, arguments) = raw_tool_call(&tokens(&["wifi_scan"])).unwrap();
        assert_eq!(name, "wifi_scan");
        assert!(arguments.as_object().expect("object").is_empty());

        let (_, arguments) = raw_tool_call(&tokens(&[
            "chronicle_recent",
            "--args",
            "{\"max_entries\":5}",
        ]))
        .unwrap();
        assert_eq!(arguments["max_entries"], 5);

        assert!(raw_tool_call(&[]).is_err());
        assert!(raw_tool_call(&tokens(&["wifi_scan", "--args", "[]"])).is_err());
    }
}
