use serde_json::{json, Map, Value};

use super::wifi::connectivity_properties;
use super::{output_schema, tool};
use crate::mcp_server::MAX_HISTORY_WINDOW_S;

pub(super) fn definition(protocol_version: &str) -> Value {
    let mut properties = connectivity_properties()
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    properties.extend(Map::from_iter([
        ("refresh_wifi".into(), json!({"type":"boolean"})),
        ("include_ble".into(), json!({"type":"boolean"})),
        (
            "ble_scan_ms".into(),
            json!({"type":"integer","minimum":500,"maximum":30000}),
        ),
        (
            "sensor_id".into(),
            json!({"type":"string","minLength":1,"maxLength":128}),
        ),
        (
            "zone".into(),
            json!({"type":"string","minLength":1,"maxLength":128}),
        ),
        (
            "movement_session".into(),
            json!({"type":"string","minLength":1,"maxLength":128}),
        ),
        ("sensor_is_moving".into(), json!({"type":"boolean"})),
        (
            "history_within_seconds".into(),
            json!({"type":"integer","minimum":1,"maximum":MAX_HISTORY_WINDOW_S}),
        ),
        (
            "history_max_events".into(),
            json!({"type":"integer","minimum":1,"maximum":2000}),
        ),
        (
            "chronicle_max_entries".into(),
            json!({"type":"integer","minimum":1,"maximum":1000}),
        ),
    ]));
    tool(
        protocol_version,
        "diagnose_incident",
        "Diagnose radio incident",
        "One compact local incident snapshot combining Wi-Fi status and analysis, connectivity stages, platform history, recent chronicle changes, and an optional native BLE scan. Sections fail independently and report actionable errors.",
        json!({"type":"object","properties":properties,"additionalProperties":false}),
        output_schema(
            &[
                "observed_at_epoch_seconds","wifi_status","wifi_analysis","connectivity",
                "wifi_history","chronicle","ble","problems","limitations",
            ],
            json!({
                "observed_at_epoch_seconds":{"type":"integer"},
                "wifi_status":{"type":"object"},"wifi_analysis":{"type":"object"},
                "connectivity":{"type":"object"},"wifi_history":{"type":"object"},
                "chronicle":{"type":"object"},"ble":{"type":"object"},
                "problems":{"type":"array","items":{"type":"string"}},
                "limitations":{"type":"array","items":{"type":"string"}}
            }),
        ),
        false,
        false,
        true,
    )
}
