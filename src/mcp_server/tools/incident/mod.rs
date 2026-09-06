//! Composite incident tool: collect sections, then classify in core.

mod evidence;

use blazingly_json::{json, Value};
use radiochron::incident::{
    classify, BleIncidentEvidence, ChronicleEvidence, EvidenceSection, IncidentEvidence,
    WifiAnalysisEvidence, WifiInterfaceSnapshot, WifiStatusEvidence,
};
use radiochron::wlan;

use super::super::protocol::Server;
use super::super::schema::{bounded_u64, optional_bool};
use super::super::transport::RequestContext;
use super::wifi;
use evidence::{
    ble_arguments, compact_ble, connectivity_evidence, epoch_seconds, platform_history, select,
    validate_ble_arguments, CONNECTIVITY_ARGS,
};

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
    let mut evidence = IncidentEvidence::new(epoch_seconds() as i64);

    context.progress(0, 6, "reading Wi-Fi status");
    let wifi_status = match wlan::wifi_status() {
        Ok(interfaces) => {
            evidence.wifi_status = EvidenceSection::available(WifiStatusEvidence {
                interfaces: interfaces
                    .iter()
                    .map(|status| WifiInterfaceSnapshot {
                        guid: status.interface.guid.clone(),
                        description: status.interface.description.clone(),
                        state: status.interface.state.clone(),
                        connected: status.connection.is_some(),
                        ssid: status
                            .connection
                            .as_ref()
                            .and_then(|connection| connection.ssid.clone()),
                        bssid: status
                            .connection
                            .as_ref()
                            .and_then(|connection| connection.bssid.clone()),
                        signal_quality: status
                            .connection
                            .as_ref()
                            .map(|connection| connection.signal_quality),
                        rssi_dbm_estimate: status
                            .connection
                            .as_ref()
                            .map(|connection| connection.rssi_dbm_estimate),
                    })
                    .collect(),
            });
            json!({"ok":true,"data":{"interfaces":interfaces}})
        }
        Err(error) => {
            let message = error.to_string();
            problems.push(format!("wifi_status: {message}"));
            evidence.wifi_status = EvidenceSection::failed("collector", message.clone());
            json!({"ok":false,"error":message})
        }
    };

    context.check_cancelled()?;
    context.progress(1, 6, "analyzing Wi-Fi environment");
    let wifi_analysis = match wifi::analyze_environment(&json!({"refresh_scan":refresh_wifi})) {
        Ok(data) => {
            evidence.wifi_analysis = EvidenceSection::available(WifiAnalysisEvidence {
                bss_count: data
                    .pointer("/analysis/bss_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize,
                finding_ids: data
                    .pointer("/analysis/findings")
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
            problems.push(format!("wifi_analysis: {message}"));
            evidence.wifi_analysis = EvidenceSection::failed("collector", message.clone());
            json!({"ok":false,"error":message})
        }
    };

    context.check_cancelled()?;
    context.progress(2, 6, "diagnosing connectivity stages");
    let connectivity = match wifi::diagnose_connectivity(&connectivity_arguments) {
        Ok(data) => {
            evidence.connectivity = EvidenceSection::available(connectivity_evidence(&data));
            json!({"ok":true,"data":data})
        }
        Err(error) => {
            let message = error.to_string();
            problems.push(format!("connectivity: {message}"));
            evidence.connectivity = EvidenceSection::failed("collector", message.clone());
            json!({"ok":false,"error":message})
        }
    };

    context.check_cancelled()?;
    context.progress(3, 6, "reading platform Wi-Fi history");
    let wifi_history = platform_history(arguments, &mut problems, &mut evidence);

    context.check_cancelled()?;
    context.progress(4, 6, "reading recent chronicle");
    let chronicle = match server.chronicle.recent(chronicle_max) {
        Ok(data) => {
            let entries = data.get("entries").and_then(Value::as_array);
            evidence.chronicle = EvidenceSection::available(ChronicleEvidence {
                entry_count: entries.map(Vec::len).unwrap_or(0),
                recent_event_ids: entries
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|entry| {
                                entry
                                    .get("event_id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                            })
                            .take(32)
                            .collect()
                    })
                    .unwrap_or_default(),
            });
            json!({"ok":true,"data":data})
        }
        Err(error) => {
            let message = error.to_string();
            problems.push(format!("chronicle: {message}"));
            evidence.chronicle = EvidenceSection::failed("collector", message.clone());
            json!({"ok":false,"error":message})
        }
    };

    context.check_cancelled()?;
    context.progress(5, 6, "scanning Bluetooth Low Energy");
    let ble = if include_ble {
        match server.ble.scan(&ble_arguments(arguments), context) {
            Ok(data) => {
                let compact = compact_ble(data);
                evidence.ble = EvidenceSection::available(BleIncidentEvidence {
                    advertisement_count: compact
                        .get("devices")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                        .unwrap_or(0),
                    finding_ids: Vec::new(),
                });
                json!({"ok":true,"included":true,"data":compact})
            }
            Err(error) => {
                let message = error.to_string();
                problems.push(format!("ble: {message}"));
                evidence.ble = EvidenceSection::failed("collector", message.clone());
                json!({"ok":false,"included":true,"error":message})
            }
        }
    } else {
        evidence.ble = EvidenceSection::NotRequested;
        json!({"ok":true,"included":false,"data":null})
    };

    context.progress(6, 6, "classifying incident");
    let incident = classify(&evidence);
    let mut limitations = incident.limitations.clone();
    limitations.extend([
        "RSSI is signal evidence, not physical distance or direction.".into(),
        "Private BLE addresses may rotate; only protocol or caller-provided identities support strong recurrence and clone evidence.".into(),
        "Native BLE scan observes advertisements and never connects to peripherals.".into(),
        "A successful association does not prove Internet reachability unless explicit connectivity targets are supplied.".into(),
    ]);

    Ok(json!({
        "observed_at_epoch_seconds": epoch_seconds(),
        "incident": incident,
        "wifi_status": wifi_status,
        "wifi_analysis": wifi_analysis,
        "connectivity": connectivity,
        "wifi_history": wifi_history,
        "chronicle": chronicle,
        "ble": ble,
        "problems": problems,
        "limitations": limitations
    }))
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
