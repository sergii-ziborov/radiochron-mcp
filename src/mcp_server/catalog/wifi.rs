use serde_json::{json, Value};

use super::{output_schema, tool};
use crate::mcp_server::MAX_HISTORY_WINDOW_S;

pub(super) fn definitions(protocol_version: &str) -> Vec<Value> {
    let mut tools = vec![
        tool(
            protocol_version,
            "wifi_status",
            "Wi-Fi status",
            "Current state of every WLAN interface.",
            empty_input(),
            output_schema(&["interfaces"], json!({
                "interfaces":{"type":"array","items":{"type":"object"}}
            })),
            true,
            true,
            false,
        ),
        tool(
            protocol_version,
            "wifi_networks",
            "Visible Wi-Fi networks",
            "Nearby BSS records with real dBm, security, channel width and load. refresh_scan initiates a standard radio scan.",
            json!({"type":"object","properties":{"refresh_scan":{"type":"boolean"},"detail":{"type":"string","enum":["summary","full"]}},"additionalProperties":false}),
            output_schema(&["count","detail","interface_errors","networks"], json!({
                "count":{"type":"integer","minimum":0},
                "detail":{"type":"string","enum":["summary","full"]},
                "cache_age_seconds":{"type":["integer","null"],"minimum":0},
                "refresh":{"type":["object","null"]},
                "interface_errors":{"type":"array","items":{"type":"object"}},
                "networks":{"type":"array","items":{"type":"object"}}
            })),
            false,
            true,
            false,
        ),
        tool(
            protocol_version,
            "wifi_analyze",
            "Analyze Wi-Fi environment",
            "Caveated findings for signal, contention, roaming candidates and security.",
            json!({"type":"object","properties":{"refresh_scan":{"type":"boolean"}},"additionalProperties":false}),
            output_schema(&["interface_errors","analysis"], json!({
                "cache_age_seconds":{"type":["integer","null"],"minimum":0},
                "refresh":{"type":["object","null"]},
                "interface_errors":{"type":"array","items":{"type":"object"}},
                "analysis":{"type":"object"}
            })),
            false,
            true,
            false,
        ),
        sample(protocol_version),
        scan(protocol_version),
        connectivity(protocol_version),
    ];
    if cfg!(windows) {
        tools.insert(3, history(protocol_version));
    }
    tools
}

fn sample(protocol_version: &str) -> Value {
    tool(
        protocol_version,
        "wifi_sample",
        "Sample Wi-Fi connection",
        "Cancelable sampling with RSSI/rate/roaming aggregates and optional interface selection.",
        json!({"type":"object","properties":{"interface_guid":{"type":"string"},"duration_seconds":{"type":"integer","minimum":1,"maximum":120},"interval_ms":{"type":"integer","minimum":250,"maximum":60000}},"additionalProperties":false}),
        output_schema(
            &[
                "duration_s",
                "interval_ms",
                "sample_count",
                "interface_guid",
                "ssid",
                "disconnected_samples",
                "failed_samples",
                "bssids_seen",
                "roam_count",
                "rssi_min_dbm",
                "rssi_max_dbm",
                "rssi_mean_dbm",
                "rssi_swing_db",
                "rx_rate_min_kbps",
                "rx_rate_max_kbps",
                "samples",
            ],
            json!({
                "duration_s":{"type":"integer"},"interval_ms":{"type":"integer"},
                "sample_count":{"type":"integer"},"interface_guid":{"type":["string","null"]},
                "ssid":{"type":["string","null"]},"disconnected_samples":{"type":"integer"},
                "failed_samples":{"type":"integer"},"bssids_seen":{"type":"array","items":{"type":"string"}},
                "roam_count":{"type":"integer"},"rssi_min_dbm":{"type":["integer","null"]},
                "rssi_max_dbm":{"type":["integer","null"]},"rssi_mean_dbm":{"type":["number","null"]},
                "rssi_swing_db":{"type":["integer","null"]},"rx_rate_min_kbps":{"type":["integer","null"]},
                "rx_rate_max_kbps":{"type":["integer","null"]},"samples":{"type":"array","items":{"type":"object"}}
            }),
        ),
        true,
        false,
        false,
    )
}

fn scan(protocol_version: &str) -> Value {
    tool(
        protocol_version,
        "wifi_scan",
        "Refresh Wi-Fi scan",
        "Initiate a standard Wi-Fi scan and wait for per-interface completion notifications.",
        empty_input(),
        output_schema(
            &[
                "requested",
                "completed",
                "failed",
                "timed_out",
                "interfaces",
            ],
            json!({
                "requested":{"type":"integer"},"completed":{"type":"integer"},
                "failed":{"type":"integer"},"timed_out":{"type":"integer"},
                "elapsed_ms":{"type":"integer"},"observed_at_epoch_seconds":{"type":"integer"},
                "interfaces":{"type":"array","items":{"type":"object"}}
            }),
        ),
        false,
        false,
        false,
    )
}

fn history(protocol_version: &str) -> Value {
    tool(
        protocol_version,
        "wifi_history",
        "Wi-Fi event history",
        "Windows WLAN AutoConfig history and evidence-based verdicts.",
        json!({"type":"object","properties":{"within_seconds":{"type":"integer","minimum":1,"maximum":MAX_HISTORY_WINDOW_S},"max_events":{"type":"integer","minimum":1,"maximum":2000},"include_events":{"type":"boolean"}},"additionalProperties":false}),
        output_schema(
            &["window_seconds", "event_count", "verdict", "events"],
            json!({
                "window_seconds":{"type":"integer","minimum":1},"event_count":{"type":"integer","minimum":0},
                "verdict":{"type":"object"},"events":{"type":["array","null"],"items":{"type":"object"}}
            }),
        ),
        true,
        true,
        false,
    )
}

fn connectivity(protocol_version: &str) -> Value {
    tool(
        protocol_version,
        "connectivity_diagnose",
        "Diagnose network connectivity",
        "Separate radio, authentication, DHCP, gateway, DNS, TCP, captive portal, TLS, packet quality and Internet reachability. No target is contacted unless supplied.",
        connectivity_input_schema(),
        output_schema(
            &["observed_at_epoch_seconds","radio","authentication","dhcp","gateway","dns","tcp","captive_portal","tls","packet_quality","internet"],
            json!({
                "observed_at_epoch_seconds":{"type":"integer"},"interface_id":{"type":["string","null"]},
                "radio":{"type":"object"},"authentication":{"type":"object"},"dhcp":{"type":"object"},
                "ip_configuration":{"type":["object","null"]},"gateway":{"type":"object"},"dns":{"type":"object"},
                "tcp":{"type":"object"},"captive_portal":{"type":"object"},"tls":{"type":"object"},
                "packet_quality":{"type":"object"},"packet_quality_measurement":{"type":["object","null"]},
                "internet":{"type":"object"}
            }),
        ),
        true,
        true,
        true,
    )
}

pub(super) fn connectivity_properties() -> Value {
    json!({
        "dns_name":{"type":"string","minLength":1,"maxLength":253},
        "tcp_target":{"type":"string","minLength":3,"maxLength":512},
        "internet_target":{"type":"string","minLength":3,"maxLength":512},
        "captive_portal_url":{"type":"string","minLength":8,"maxLength":2048},
        "captive_portal_expected_status":{"type":"integer","minimum":100,"maximum":599},
        "tls_target":{"type":"string","minLength":3,"maxLength":512},
        "quality_target":{"type":"string","minLength":3,"maxLength":512},
        "quality_attempts":{"type":"integer","minimum":1,"maximum":20},
        "timeout_ms":{"type":"integer","minimum":100,"maximum":30000}
    })
}

fn connectivity_input_schema() -> Value {
    json!({"type":"object","properties":connectivity_properties(),"additionalProperties":false})
}

fn empty_input() -> Value {
    json!({"type":"object","properties":{},"additionalProperties":false})
}
