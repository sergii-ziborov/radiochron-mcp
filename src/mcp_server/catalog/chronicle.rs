use serde_json::{json, Value};

use super::{output_schema, tool};

pub(super) fn definitions(protocol_version: &str) -> Vec<Value> {
    let status = output_schema(
        &["running", "path"],
        json!({
            "running":{"type":"boolean"},"path":{"type":"string"},
            "started_at_epoch_seconds":{"type":["integer","null"]},
            "stopped_at_epoch_seconds":{"type":["integer","null"]},
            "entries_written_this_run":{"type":"integer"},
            "last_error":{"type":["string","null"]}
        }),
    );
    vec![
        tool(
            protocol_version,
            "chronicle_start",
            "Start RadioChron chronicle",
            "Start the local change-only JSONL recorder in the platform application-state directory.",
            json!({"type":"object","properties":{"interval_seconds":{"type":"integer","minimum":1,"maximum":300},"signal_threshold_db":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false}),
            status.clone(),
            false,
            false,
            false,
        ),
        tool(
            protocol_version,
            "chronicle_stop",
            "Stop RadioChron chronicle",
            "Stop and flush the process-local recorder.",
            empty_input(),
            status.clone(),
            false,
            false,
            false,
        ),
        tool(
            protocol_version,
            "chronicle_status",
            "Chronicle status",
            "Read recorder state and storage path.",
            empty_input(),
            status,
            true,
            true,
            false,
        ),
        tool(
            protocol_version,
            "chronicle_recent",
            "Recent chronicle changes",
            "Read recent change-only entries across active and rotated JSONL files.",
            json!({"type":"object","properties":{"max_entries":{"type":"integer","minimum":1,"maximum":1000}},"additionalProperties":false}),
            output_schema(&["path","count","entries"], json!({
                "path":{"type":"string"},"count":{"type":"integer"},
                "invalid_lines":{"type":"integer"},"entries":{"type":"array","items":{"type":"object"}}
            })),
            true,
            true,
            false,
        ),
    ]
}

fn empty_input() -> Value {
    json!({"type":"object","properties":{},"additionalProperties":false})
}
