mod incident;
mod wifi;

use std::time::Duration;

use blazingly_json::{json, Value};
use mcport::{MethodReply, ServerIdentity, ToolReply, ToolServer};
use radiochron::wlan;

use super::protocol::Server;
use super::schema::{
    bounded_i32, bounded_u64, error_response, reject_unknown_arguments, rpc_error,
    rpc_error_with_data, tool_result, RpcError,
};
use super::transport::RequestContext;
use super::{catalog, resources};
use super::{INTERNAL_ERROR, INVALID_PARAMS, SCAN_TIMEOUT};

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

const INCIDENT_ARGS: &[&str] = &[
    "refresh_wifi",
    "include_ble",
    "ble_scan_ms",
    "sensor_id",
    "zone",
    "movement_session",
    "sensor_is_moving",
    "history_within_seconds",
    "history_max_events",
    "chronicle_max_entries",
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

pub(super) struct McportToolServer<'a> {
    server: &'a Server,
    context: &'a RequestContext,
    protocol_version: &'static str,
}

impl<'a> McportToolServer<'a> {
    pub(super) const fn new(
        server: &'a Server,
        context: &'a RequestContext,
        protocol_version: &'static str,
    ) -> Self {
        Self {
            server,
            context,
            protocol_version,
        }
    }
}

impl ToolServer for McportToolServer<'_> {
    fn identity(&self) -> ServerIdentity {
        ServerIdentity::new(
            "radiochron",
            env!("CARGO_PKG_VERSION"),
            "Local Wi-Fi incident diagnostics and native BLE observation/history.",
        )
    }

    fn catalog(&mut self) -> Value {
        catalog::tool_definitions(self.protocol_version)
    }

    fn capabilities(&self) -> Value {
        json!({
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false}
        })
    }

    fn has_tool(&self, name: &str) -> Option<bool> {
        Some(allowed_arguments(name).is_some())
    }

    fn call(&mut self, name: &str, arguments: Value) -> ToolReply {
        let Some(allowed) = allowed_arguments(name) else {
            return ToolReply::error(format!("Unknown tool: {name}"));
        };
        if let Err(error) = reject_unknown_arguments(&arguments, allowed) {
            return ToolReply::error(error.to_string());
        }
        match execute(self.server, name, &arguments, self.context) {
            Ok(value) => ToolReply::structured(value),
            Err(error) => ToolReply::error(error.to_string()),
        }
    }

    fn call_method(&mut self, method: &str, params: Value) -> Option<MethodReply> {
        let outcome = match method {
            "resources/list" => Ok(json!({
                "resources": resources::definitions()
            })),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "resources/read" => resources::read(self.server, &params),
            _ => return None,
        };
        Some(match outcome {
            Ok(value) => MethodReply::Success(value),
            Err(error) => MethodReply::Error {
                code: error.code,
                message: error.message,
                data: error.data,
            },
        })
    }
}

pub(super) fn handle_mcport_line(
    server: &Server,
    line: &str,
    context: &RequestContext,
) -> Option<String> {
    let protocol_version = match server.ready_protocol_version() {
        Ok(protocol_version) => protocol_version,
        Err(_) => return server.handle_line(line, context),
    };
    let mut tools = McportToolServer::new(server, context, protocol_version);
    let mut response = Vec::with_capacity(4096);
    match mcport::serve_message(&mut tools, line, &mut response) {
        Ok(true) => {
            if response.last() == Some(&b'\n') {
                response.pop();
            }
            String::from_utf8(response).ok()
        }
        Ok(false) => None,
        Err(error) => Some(error_response(
            Value::Null,
            rpc_error_with_data(
                INTERNAL_ERROR,
                "mcport response failure",
                json!({"detail": error.to_string()}),
            ),
        )),
    }
}

pub(super) fn call(
    server: &Server,
    params: &Value,
    context: &RequestContext,
) -> Result<Value, RpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, "tools/call requires a name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !arguments.is_object() {
        return Err(rpc_error(
            INVALID_PARAMS,
            "tool arguments must be an object",
        ));
    }
    let Some(allowed) = allowed_arguments(name) else {
        return Err(rpc_error(INVALID_PARAMS, format!("Unknown tool: {name}")));
    };
    if let Err(error) = reject_unknown_arguments(&arguments, allowed) {
        return Ok(tool_result(Err(error)));
    }

    let outcome = execute(server, name, &arguments, context);
    Ok(tool_result(outcome))
}

fn allowed_arguments(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "wifi_status" | "wifi_scan" | "chronicle_stop" | "chronicle_status" | "ble_histories" => {
            Some(&[])
        }
        "wifi_networks" => Some(&["refresh_scan", "detail"]),
        "wifi_analyze" => Some(&["refresh_scan"]),
        "wifi_history" => Some(&["within_seconds", "max_events", "include_events"]),
        "wifi_sample" => Some(&["interface_guid", "duration_seconds", "interval_ms"]),
        "connectivity_diagnose" => Some(CONNECTIVITY_ARGS),
        "chronicle_start" => Some(&["interval_seconds", "signal_threshold_db"]),
        "chronicle_recent" => Some(&["max_entries"]),
        "ble_scan" => Some(&[
            "duration_ms",
            "sensor_id",
            "zone",
            "movement_session",
            "sensor_is_moving",
        ]),
        "ble_identify" => Some(&["advertisement"]),
        "ble_tracker_reset" => Some(&["policy"]),
        "ble_observe" => Some(&["observation"]),
        "ble_evaluate" => Some(&["now_ms"]),
        "diagnose_incident" => Some(INCIDENT_ARGS),
        _ => None,
    }
}

fn execute(
    server: &Server,
    name: &str,
    arguments: &Value,
    context: &RequestContext,
) -> anyhow::Result<Value> {
    match name {
        "wifi_status" => wlan::wifi_status().map(|interfaces| json!({"interfaces":interfaces})),
        "wifi_scan" => wlan::bss::scan_and_wait(SCAN_TIMEOUT)
            .and_then(|refresh| Ok(blazingly_json::to_value(refresh)?)),
        "wifi_networks" => wifi::collect_networks(arguments),
        "wifi_analyze" => wifi::analyze_environment(arguments),
        "wifi_history" => wifi::history(arguments),
        "wifi_sample" => wifi::sample(arguments, context),
        "connectivity_diagnose" => wifi::diagnose_connectivity(arguments),
        "ble_scan" => server.ble.scan(arguments, context),
        "ble_identify" => server.ble.identify(arguments),
        "ble_tracker_reset" => server.ble.reset(arguments),
        "ble_observe" => server.ble.observe(arguments),
        "ble_histories" => server.ble.histories(),
        "ble_evaluate" => server.ble.evaluate(arguments),
        "chronicle_start" => {
            let interval = bounded_u64(arguments, "interval_seconds", 5, 1, 300)?;
            let threshold = bounded_i32(arguments, "signal_threshold_db", 8, 1, 50)?;
            server
                .chronicle
                .start(Duration::from_secs(interval), threshold)
        }
        "chronicle_stop" => server.chronicle.stop(),
        "chronicle_status" => Ok(server.chronicle.status()),
        "chronicle_recent" => {
            let max = bounded_u64(arguments, "max_entries", 100, 1, 1000)? as usize;
            server.chronicle.recent(max)
        }
        "diagnose_incident" => incident::diagnose(server, arguments, context),
        _ => unreachable!("tool names are validated before execution"),
    }
}
