use std::time::{SystemTime, UNIX_EPOCH};

use radiochron::wlan;
use serde_json::{json, Map, Value};

use super::super::protocol::Server;
use super::super::schema::{bounded_optional_string, bounded_u64, optional_bool};
use super::super::transport::RequestContext;
use super::wifi;

const CONNECTIVITY_ARGS: &[&str] = &[
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

pub(super) fn diagnose(
    server: &Server,
    arguments: &Value,
    context: &RequestContext,
) -> anyhow::Result<Value> {
    context.check_cancelled()?;
    let refresh_wifi = optional_bool(arguments, "refresh_wifi", false)?;
    let include_ble = optional_bool(arguments, "include_ble", true)?;
    let chronicle_max = bounded_u64(arguments, "chronicle_max_entries", 100, 1, 1000)? as usize;
    let _ = bounded_u64(
        arguments,
        "history_within_seconds",
        3600,
        1,
        super::super::MAX_HISTORY_WINDOW_S,
    )?;
    let _ = bounded_u64(arguments, "history_max_events", 200, 1, 2000)?;
    let connectivity_arguments = select(arguments, CONNECTIVITY_ARGS);
    wifi::validate_connectivity(&connectivity_arguments)?;
    if include_ble {
        validate_ble_arguments(arguments)?;
    }
    let mut problems = Vec::new();

    context.progress(0, 6, "reading Wi-Fi status");
    let wifi_status = section(
        "wifi_status",
        wlan::wifi_status().map(|interfaces| json!({"interfaces":interfaces})),
        &mut problems,
    );

    context.check_cancelled()?;
    context.progress(1, 6, "analyzing Wi-Fi environment");
    let wifi_analysis = section(
        "wifi_analysis",
        wifi::analyze_environment(&json!({"refresh_scan":refresh_wifi})),
        &mut problems,
    );

    context.check_cancelled()?;
    context.progress(2, 6, "diagnosing connectivity stages");
    let connectivity = section(
        "connectivity",
        wifi::diagnose_connectivity(&connectivity_arguments),
        &mut problems,
    );

    context.check_cancelled()?;
    context.progress(3, 6, "reading platform Wi-Fi history");
    let wifi_history = platform_history(arguments, &mut problems);

    context.check_cancelled()?;
    context.progress(4, 6, "reading recent chronicle");
    let chronicle = section(
        "chronicle",
        server.chronicle.recent(chronicle_max),
        &mut problems,
    );

    context.check_cancelled()?;
    context.progress(5, 6, "scanning Bluetooth Low Energy");
    let ble = if include_ble {
        section(
            "ble",
            server
                .ble
                .scan(&ble_arguments(arguments), context)
                .map(compact_ble),
            &mut problems,
        )
    } else {
        json!({"ok":true,"included":false,"data":null})
    };
    context.progress(6, 6, "incident snapshot complete");

    Ok(json!({
        "observed_at_epoch_seconds": epoch_seconds(),
        "wifi_status": wifi_status,
        "wifi_analysis": wifi_analysis,
        "connectivity": connectivity,
        "wifi_history": wifi_history,
        "chronicle": chronicle,
        "ble": ble,
        "problems": problems,
        "limitations": [
            "RSSI is signal evidence, not physical distance or direction.",
            "Private BLE addresses may rotate; only protocol or caller-provided identities support strong recurrence and clone evidence.",
            "Native BLE scan observes advertisements and never connects to peripherals.",
            "A successful association does not prove Internet reachability unless explicit connectivity targets are supplied."
        ]
    }))
}

fn section(name: &str, result: anyhow::Result<Value>, problems: &mut Vec<String>) -> Value {
    match result {
        Ok(data) => json!({"ok":true,"data":data}),
        Err(error) => {
            let message = error.to_string();
            problems.push(format!("{name}: {message}"));
            json!({"ok":false,"error":message})
        }
    }
}

#[cfg(windows)]
fn platform_history(arguments: &Value, problems: &mut Vec<String>) -> Value {
    let result = (|| -> anyhow::Result<Value> {
        let history_arguments = json!({
            "within_seconds": bounded_u64(
                arguments,
                "history_within_seconds",
                3600,
                1,
                super::super::MAX_HISTORY_WINDOW_S,
            )?,
            "max_events": bounded_u64(arguments, "history_max_events", 200, 1, 2000)?,
            "include_events": false
        });
        wifi::history(&history_arguments)
    })();
    section("wifi_history", result, problems)
}

#[cfg(not(windows))]
fn platform_history(_arguments: &Value, _problems: &mut Vec<String>) -> Value {
    json!({
        "ok": true,
        "available": false,
        "reason": "native WLAN event history is currently available on Windows only",
        "data": null
    })
}

fn select(arguments: &Value, names: &[&str]) -> Value {
    let mut selected = Map::new();
    for name in names {
        if let Some(value) = arguments.get(*name) {
            selected.insert((*name).to_string(), value.clone());
        }
    }
    Value::Object(selected)
}

fn ble_arguments(arguments: &Value) -> Value {
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

fn validate_ble_arguments(arguments: &Value) -> anyhow::Result<()> {
    let _ = bounded_u64(arguments, "ble_scan_ms", 4_000, 500, 30_000)?;
    for name in ["sensor_id", "zone", "movement_session"] {
        let _ = bounded_optional_string(arguments, name, 128)?;
    }
    let _ = optional_bool(arguments, "sensor_is_moving", false)?;
    Ok(())
}

fn compact_ble(mut value: Value) -> Value {
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

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_only_copies_connectivity_arguments() {
        let value = json!({"dns_name":"example.com","include_ble":false});
        assert_eq!(
            select(&value, CONNECTIVITY_ARGS),
            json!({"dns_name":"example.com"})
        );
    }

    #[test]
    fn compact_ble_removes_addresses_and_payload_bytes() {
        let value = json!({"devices":[{
            "advertisement":{
                "address":"private",
                "local_name":"beacon",
                "address_type":"unknown",
                "rssi_dbm":-40,
                "tx_power_dbm":null,
                "service_uuids":["feaa"],
                "manufacturer_data":[{"company_id":76,"data":[1,2,3]}]
            },
            "identity":{"key":"opaque"}
        }]});
        let compact = compact_ble(value);
        assert!(compact["devices"][0].get("advertisement").is_none());
        assert!(compact.to_string().find("private").is_none());
        assert_eq!(compact["devices"][0]["radio"]["manufacturer_ids"][0], 76);
    }
}
