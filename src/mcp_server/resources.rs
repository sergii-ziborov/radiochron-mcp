use radiochron::wlan;
use radiochron::wlan::bss::BssSummary;
use serde_json::{json, Value};

use super::protocol::Server;
use super::schema::{encode, rpc_error, RpcError};
use super::{INTERNAL_ERROR, INVALID_PARAMS, RESOURCE_NOT_FOUND};

pub(super) const URI_REPORT_MD: &str = "radiochron://report/latest";
pub(super) const URI_REPORT_JSON: &str = "radiochron://report/latest.json";
pub(super) const URI_STATUS: &str = "radiochron://status";
pub(super) const URI_NETWORKS: &str = "radiochron://networks";
pub(super) const URI_CHRONICLE: &str = "radiochron://chronicle/recent";
pub(super) const URI_BLE_HISTORIES: &str = "radiochron://ble/histories";

pub(super) fn definitions() -> Value {
    json!([
        {"uri":URI_REPORT_MD,"name":"wifi_diagnostic_report","title":"Wi-Fi Diagnostic Report","description":"Current cached diagnostic report.","mimeType":"text/markdown","annotations":{"audience":["user","assistant"],"priority":0.9}},
        {"uri":URI_REPORT_JSON,"name":"wifi_diagnostic_report_json","title":"Wi-Fi Diagnostic Report (JSON)","description":"Machine-readable cached report.","mimeType":"application/json"},
        {"uri":URI_STATUS,"name":"wifi_status","title":"Current Wi-Fi Connection","description":"Per-interface association state.","mimeType":"application/json"},
        {"uri":URI_NETWORKS,"name":"wifi_networks","title":"Visible Networks","description":"Compact cached BSS list.","mimeType":"application/json"},
        {"uri":URI_CHRONICLE,"name":"chronicle_recent","title":"Recent Radio Changes","description":"Recent entries from the local change-only chronicle.","mimeType":"application/json"},
        {"uri":URI_BLE_HISTORIES,"name":"ble_histories","title":"BLE Histories","description":"Process-local BLE identity history from native scans and explicit observations.","mimeType":"application/json"}
    ])
}

pub(super) fn read(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, "resources/read requires a uri"))?;
    let (mime, body) = match uri {
        URI_REPORT_MD => ("text/markdown", render(Format::Markdown)),
        URI_REPORT_JSON => ("application/json", render(Format::Json)),
        URI_STATUS => ("application/json", render(Format::Status)),
        URI_NETWORKS => ("application/json", render(Format::Networks)),
        URI_CHRONICLE => (
            "application/json",
            server
                .chronicle
                .recent(100)
                .and_then(|value| encode(&value)),
        ),
        URI_BLE_HISTORIES => (
            "application/json",
            server.ble.histories().and_then(|value| encode(&value)),
        ),
        other => {
            return Err(rpc_error(
                RESOURCE_NOT_FOUND,
                format!("resource not found: {other}"),
            ));
        }
    };
    let text = body.map_err(|error| rpc_error(INTERNAL_ERROR, error.to_string()))?;
    Ok(json!({
        "contents": [{"uri": uri, "mimeType": mime, "text": text}]
    }))
}

enum Format {
    Markdown,
    Json,
    Status,
    Networks,
}

fn render(format: Format) -> anyhow::Result<String> {
    let status = wlan::wifi_status()?;
    match format {
        Format::Status => encode(&json!({ "interfaces": status })),
        Format::Networks => {
            let collection = wlan::bss::bss_list_detailed()?;
            encode(&json!({
                "cache_age_seconds": wlan::bss::last_refresh_age_seconds(),
                "interface_errors": collection.interface_errors,
                "networks": collection.entries.iter().map(BssSummary::from).collect::<Vec<_>>()
            }))
        }
        Format::Markdown | Format::Json => render_report(format, status),
    }
}

fn render_report(format: Format, status: Vec<wlan::WifiStatus>) -> anyhow::Result<String> {
    let collection = wlan::bss::bss_list_detailed()?;
    let entries = &collection.entries;
    let connection = status.iter().find_map(|entry| entry.connection.as_ref());
    let analysis = wlan::analyze::analyze(entries, connection);
    match format {
        Format::Markdown => {
            let mut report = crate::report::markdown(&status, entries, &analysis);
            if !collection.interface_errors.is_empty() {
                report.push_str("\n> Scan completeness warning: one or more interfaces failed; see the JSON resource for native error codes.\n");
            }
            Ok(report)
        }
        Format::Json => {
            let mut report = crate::report::json(&status, entries, &analysis);
            if let Some(object) = report.as_object_mut() {
                object.insert(
                    "scan_interface_errors".to_string(),
                    serde_json::to_value(collection.interface_errors)?,
                );
            }
            encode(&report)
        }
        _ => unreachable!("report formats are filtered by the caller"),
    }
}
