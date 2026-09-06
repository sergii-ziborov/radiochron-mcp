//! Evidence normalisation helpers for the incident tool.

use std::time::{SystemTime, UNIX_EPOCH};

use blazingly_json::{json, Map, Value};
use radiochron::incident::{ConnectivityEvidence, EvidenceSection, IncidentEvidence};

use super::super::super::schema::{bounded_optional_string, bounded_u64, optional_bool};

#[cfg(windows)]
use super::super::wifi;
#[cfg(windows)]
use radiochron::incident::HistoryEvidence;

pub(super) const CONNECTIVITY_ARGS: &[&str] = &[
    "dns_name",
    "tcp_target",
    "internet_target",
    "captive_portal_url",
    "captive_portal_expected_status",
    "tls_target",
    "quality_target",
    "quality_attempts",
    "timeout_ms",
];

pub(super) fn connectivity_evidence(data: &Value) -> ConnectivityEvidence {
    let stages = [
        "radio",
        "authentication",
        "dhcp",
        "gateway",
        "dns",
        "tcp",
        "captive_portal",
        "tls",
        "packet_quality",
        "internet",
    ];
    let mut failed_stages = Vec::new();
    let mut unknown_stages = Vec::new();
    for stage in stages {
        match data
            .get(stage)
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
        {
            Some("fail") => failed_stages.push(stage.to_string()),
            Some("unknown") => unknown_stages.push(stage.to_string()),
            _ => {}
        }
    }
    ConnectivityEvidence {
        failed_stages,
        unknown_stages,
    }
}

pub(super) fn platform_history(
    arguments: &Value,
    problems: &mut Vec<String>,
    evidence: &mut IncidentEvidence,
) -> Value {
    #[cfg(windows)]
    {
        let result = (|| -> anyhow::Result<Value> {
            let history_arguments = json!({
                "within_seconds": bounded_u64(
                    arguments,
                    "history_within_seconds",
                    3600,
                    1,
                    crate::mcp_server::MAX_HISTORY_WINDOW_S,
                )?,
                "max_events": bounded_u64(arguments, "history_max_events", 200, 1, 2000)?,
                "include_events": false
            });
            wifi::history(&history_arguments)
        })();
        match result {
            Ok(data) => {
                evidence.history = EvidenceSection::available(HistoryEvidence {
                    events_considered: data
                        .pointer("/verdict/events_considered")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as usize,
                    finding_ids: data
                        .pointer("/verdict/findings")
                        .and_then(Value::as_array)
                        .map(|findings| {
                            findings
                                .iter()
                                .filter_map(|finding| {
                                    finding
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .map(str::to_string)
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                });
                json!({"ok":true,"data":data})
            }
            Err(error) => {
                let message = error.to_string();
                problems.push(format!("wifi_history: {message}"));
                evidence.history = EvidenceSection::failed("collector", message.clone());
                json!({"ok":false,"error":message})
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (arguments, problems);
        evidence.history = EvidenceSection::unavailable(
            "native WLAN event history is currently available on Windows only",
        );
        json!({
            "ok": true,
            "available": false,
            "reason": "native WLAN event history is currently available on Windows only",
            "data": null
        })
    }
}

pub(super) fn select(arguments: &Value, names: &[&str]) -> Value {
    let mut selected = Map::new();
    for name in names {
        if let Some(value) = arguments.get(*name) {
            selected.insert((*name).to_string(), value.clone());
        }
    }
    Value::Object(selected)
}

pub(super) fn ble_arguments(arguments: &Value) -> Value {
    let mut selected = Map::new();
    if let Some(value) = arguments.get("ble_scan_ms") {
        selected.insert("duration_ms".into(), value.clone());
    }
    for name in ["sensor_id", "zone", "movement_session", "sensor_is_moving"] {
        if let Some(value) = arguments.get(name) {
            selected.insert(name.into(), value.clone());
        }
    }
    Value::Object(selected)
}

pub(super) fn validate_ble_arguments(arguments: &Value) -> anyhow::Result<()> {
    let _ = bounded_u64(arguments, "ble_scan_ms", 4_000, 500, 30_000)?;
    for name in ["sensor_id", "zone", "movement_session"] {
        let _ = bounded_optional_string(arguments, name, 128)?;
    }
    let _ = optional_bool(arguments, "sensor_is_moving", false)?;
    Ok(())
}

pub(super) fn compact_ble(mut value: Value) -> Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let Some(devices) = object.get_mut("devices").and_then(Value::as_array_mut) else {
        return value;
    };
    for device in devices {
        let Some(device_object) = device.as_object_mut() else {
            continue;
        };
        let advertisement = device_object.remove("advertisement").unwrap_or(Value::Null);
        device_object.insert(
            "radio".into(),
            json!({
                "name": advertisement.get("local_name").cloned().unwrap_or(Value::Null),
                "address_type": advertisement.get("address_type").cloned().unwrap_or(Value::Null),
                "rssi_dbm": advertisement.get("rssi_dbm").cloned().unwrap_or(Value::Null),
                "tx_power_dbm": advertisement.get("tx_power_dbm").cloned().unwrap_or(Value::Null),
                "service_uuids": advertisement.get("service_uuids").cloned().unwrap_or_else(|| json!([])),
                "manufacturer_ids": advertisement
                    .get("manufacturer_data")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().filter_map(|item| item.get("company_id").cloned()).collect::<Vec<_>>())
                    .unwrap_or_default()
            }),
        );
    }
    value
}

pub(super) fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
