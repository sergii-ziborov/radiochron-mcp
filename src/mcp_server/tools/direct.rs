//! The command line's way into the tool handlers.
//!
//! Kept beside the dispatcher rather than in `cli` so that the bounds, the
//! argument validation and the catalogue of names have exactly one home. A
//! command that reaches a tool goes through [`run_once`]; nothing in `cli`
//! constructs a [`Server`] or decides what a legal argument is.

use std::time::Duration;

use blazingly_json::{Map, Value};

use super::super::protocol::Server;
use super::super::schema::{bounded_i32, bounded_u64, reject_unknown_arguments};
use super::super::transport::RequestContext;
use super::{allowed_arguments, execute};

/// Execute one tool outside the JSON-RPC loop.
///
/// The CLI is a second front end over exactly these handlers rather than a
/// parallel implementation, so a command and its MCP tool cannot drift: the
/// argument names, the bounds and the error text are the ones the protocol path
/// uses. The [`Server`] is per-invocation, which is why the CLI exposes the
/// chronicle's file-backed commands and not the session-scoped start/stop pair.
pub(crate) fn run_once(name: &str, arguments: &Value) -> anyhow::Result<Value> {
    let Some(allowed) = allowed_arguments(name) else {
        anyhow::bail!("unknown tool: {name}\nAvailable: {}", TOOL_NAMES.join(", "));
    };
    reject_unknown_arguments(arguments, allowed)?;
    execute(&Server::new(), name, arguments, &RequestContext::idle())
}

/// Recorder settings for the CLI's foreground `chronicle record`.
///
/// Routed through the same bounds `chronicle_start` applies, so a command line
/// and a tool call cannot disagree about a legal interval or threshold.
pub(crate) fn recording_options(
    interval_seconds: Option<i64>,
    signal_threshold_db: Option<i64>,
) -> anyhow::Result<(Duration, i32)> {
    let mut fields = Vec::new();
    for (name, value) in [
        ("interval_seconds", interval_seconds),
        ("signal_threshold_db", signal_threshold_db),
    ] {
        if let Some(value) = value {
            fields.push((name.to_string(), Value::from(value)));
        }
    }
    recording_bounds(&Value::Object(Map::from_iter(fields)))
}

/// The one place the recorder's accepted ranges are written down.
pub(super) fn recording_bounds(arguments: &Value) -> anyhow::Result<(Duration, i32)> {
    let interval = bounded_u64(arguments, "interval_seconds", 5, 1, 300)?;
    let threshold = bounded_i32(arguments, "signal_threshold_db", 8, 1, 50)?;
    Ok((Duration::from_secs(interval), threshold))
}

/// Every tool the CLI can reach, for `radiochron tool` and its error message.
///
/// A test asserts this matches the dispatcher, so a tool added to one and not
/// the other fails the build rather than going quietly missing from the CLI.
pub(crate) const TOOL_NAMES: &[&str] = &[
    "wifi_status",
    "wifi_scan",
    "wifi_networks",
    "wifi_analyze",
    "wifi_history",
    "wifi_sample",
    "connectivity_diagnose",
    "ble_scan",
    "ble_identify",
    "ble_tracker_reset",
    "ble_observe",
    "ble_histories",
    "ble_evaluate",
    "chronicle_start",
    "chronicle_stop",
    "chronicle_status",
    "chronicle_recent",
    "diagnose_incident",
];

#[cfg(test)]
mod tests {
    use super::{allowed_arguments, recording_bounds, recording_options, run_once, TOOL_NAMES};
    use blazingly_json::json;

    #[test]
    fn every_listed_tool_is_one_the_dispatcher_accepts() {
        for name in TOOL_NAMES {
            assert!(
                allowed_arguments(name).is_some(),
                "{name} is listed but the dispatcher does not know it"
            );
        }
    }

    /// The catalogue is platform-gated — `catalog::wifi` publishes
    /// `wifi_history` only on Windows — while the dispatcher accepts it
    /// everywhere and answers with an honest "Windows only" error. So the
    /// invariant is containment, tightened to equality on the platform that
    /// publishes everything.
    #[test]
    fn every_published_tool_is_reachable_from_the_command_line() {
        let catalogue = super::super::super::catalog::tool_definitions("2025-11-25");
        let mut published: Vec<&str> = catalogue
            .as_array()
            .expect("catalogue is an array")
            .iter()
            .map(|tool| tool["name"].as_str().expect("every tool is named"))
            .collect();
        published.sort_unstable();

        for name in &published {
            assert!(
                TOOL_NAMES.contains(name),
                "{name} is published but the command line cannot reach it"
            );
        }

        if cfg!(windows) {
            let mut listed = TOOL_NAMES.to_vec();
            listed.sort_unstable();
            assert_eq!(
                published, listed,
                "Windows publishes every tool, so the two lists must match exactly"
            );
        }
    }

    #[test]
    fn unknown_tools_are_refused_before_anything_runs() {
        let error = run_once("wifi_teleport", &json!({}))
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("unknown tool"));
        assert!(error.contains("wifi_status"), "must list what is available");
    }

    #[test]
    fn unknown_arguments_are_refused_before_anything_runs() {
        let error = run_once("chronicle_recent", &json!({"maximum": 5}))
            .expect_err("must refuse")
            .to_string();
        assert!(error.contains("maximum"), "{error}");
    }

    #[test]
    fn recorder_defaults_and_bounds_are_shared_with_the_tool() {
        let (interval, threshold) = recording_options(None, None).unwrap();
        assert_eq!(interval.as_secs(), 5);
        assert_eq!(threshold, 8);

        let (interval, threshold) = recording_options(Some(30), Some(12)).unwrap();
        assert_eq!(interval.as_secs(), 30);
        assert_eq!(threshold, 12);

        assert!(recording_options(Some(0), None).is_err());
        assert!(recording_options(Some(301), None).is_err());
        assert!(recording_options(None, Some(51)).is_err());

        // The tool path reaches the identical check.
        assert!(recording_bounds(&json!({"interval_seconds": 301})).is_err());
        assert!(recording_bounds(&json!({"interval_seconds": "soon"})).is_err());
    }
}
