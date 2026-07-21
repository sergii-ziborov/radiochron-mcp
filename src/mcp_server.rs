//! Model Context Protocol server over newline-delimited JSON-RPC 2.0 stdio.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};

use radiochron::wlan;
use radiochron::wlan::bss::BssSummary;

use crate::chronicle::ChronicleService;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SCAN_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_HISTORY_WINDOW_S: u64 = 365 * 24 * 60 * 60;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

const URI_REPORT_MD: &str = "radiochron://report/latest";
const URI_REPORT_JSON: &str = "radiochron://report/latest.json";
const URI_STATUS: &str = "radiochron://status";
const URI_NETWORKS: &str = "radiochron://networks";
const URI_CHRONICLE: &str = "radiochron://chronicle/recent";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Uninitialized,
    Initializing,
    Ready,
}

struct Server {
    phase: Mutex<Phase>,
    chronicle: ChronicleService,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

struct RegisteredRequest {
    server: Arc<Server>,
    id: Value,
}

impl Drop for RegisteredRequest {
    fn drop(&mut self) {
        self.server.finish_request(&self.id);
    }
}

impl Server {
    fn new() -> Self {
        Self {
            phase: Mutex::new(Phase::Uninitialized),
            chronicle: ChronicleService::new(),
            cancellations: Mutex::new(HashMap::new()),
        }
    }

    fn handle_line(&self, line: &str, context: &RequestContext) -> Option<String> {
        let line = line.trim_start_matches('\u{feff}');
        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return Some(error_response(Value::Null, PARSE_ERROR, &error.to_string()));
            }
        };

        if !message.is_object() || message.get("jsonrpc") != Some(&Value::String("2.0".into())) {
            return Some(error_response(
                message.get("id").cloned().unwrap_or(Value::Null),
                INVALID_REQUEST,
                "request must be a JSON-RPC 2.0 object",
            ));
        }

        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "missing method",
            ));
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        // Notifications are still dispatched: initialized and cancellation
        // change server state, but JSON-RPC forbids replying to them.
        let Some(id) = id else {
            let _ = self.dispatch(method, &params, context);
            return None;
        };

        match self.dispatch(method, &params, context) {
            Ok(result) => Some(success_response(id, result)),
            Err(error) => Some(error_response(id, error.code, &error.message)),
        }
    }

    fn dispatch(
        &self,
        method: &str,
        params: &Value,
        context: &RequestContext,
    ) -> Result<Value, RpcError> {
        match method {
            "initialize" => return self.initialize(params),
            "notifications/initialized" => {
                let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
                if *phase != Phase::Initializing {
                    return Err(rpc_error(INVALID_REQUEST, "server was not initializing"));
                }
                *phase = Phase::Ready;
                return Ok(json!({}));
            }
            "notifications/cancelled" => {
                self.cancel(params)?;
                return Ok(json!({}));
            }
            "ping" => return Ok(json!({})),
            _ => {}
        }

        if *self.phase.lock().unwrap_or_else(|e| e.into_inner()) != Phase::Ready {
            return Err(rpc_error(
                INVALID_REQUEST,
                "initialize and notifications/initialized must complete first",
            ));
        }

        match method {
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => self.call_tool(params, context),
            "resources/list" => Ok(json!({ "resources": resource_definitions() })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => self.read_resource(params),
            other => Err(rpc_error(
                METHOD_NOT_FOUND,
                format!("unknown method: {other}"),
            )),
        }
    }

    fn initialize(&self, params: &Value) -> Result<Value, RpcError> {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(INVALID_PARAMS, "initialize requires protocolVersion"))?;
        let mut phase = self.phase.lock().unwrap_or_else(|e| e.into_inner());
        if *phase != Phase::Uninitialized {
            return Err(rpc_error(INVALID_REQUEST, "server is already initialized"));
        }
        *phase = Phase::Initializing;

        // This implementation exposes the 2025-06-18 shape. Per MCP version
        // negotiation, an unsupported client proposal receives our supported
        // version and the client decides whether to continue.
        let negotiated = if requested == PROTOCOL_VERSION {
            requested
        } else {
            PROTOCOL_VERSION
        };

        Ok(json!({
            "protocolVersion": negotiated,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false }
            },
            "serverInfo": {
                "name": "radiochron",
                "version": env!("CARGO_PKG_VERSION")
            },
            "instructions": "Local Windows Wi-Fi diagnostics. SSIDs, BSSIDs and MAC addresses are sensitive. Most tools are read-only; wifi_scan and refresh_scan initiate a standard radio scan, while chronicle_start/stop control a local recorder."
        }))
    }

    fn cancel(&self, params: &Value) -> Result<(), RpcError> {
        let request_id = params
            .get("requestId")
            .ok_or_else(|| rpc_error(INVALID_PARAMS, "cancellation requires requestId"))?;
        if let Some(flag) = self
            .cancellations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&request_key(request_id))
        {
            flag.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn register_request(&self, id: &Value) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.cancellations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_key(id), flag.clone());
        flag
    }

    fn finish_request(&self, id: &Value) {
        self.cancellations
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_key(id));
    }

    fn call_tool(&self, params: &Value, context: &RequestContext) -> Result<Value, RpcError> {
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

        let allowed = match name {
            "wifi_status" | "wifi_scan" | "chronicle_stop" | "chronicle_status" => &[][..],
            "wifi_networks" => &["refresh_scan", "detail"][..],
            "wifi_analyze" => &["refresh_scan"][..],
            "wifi_history" => &["within_seconds", "max_events", "include_events"][..],
            "wifi_sample" => &["interface_guid", "duration_seconds", "interval_ms"][..],
            "chronicle_start" => &["interval_seconds", "signal_threshold_db"][..],
            "chronicle_recent" => &["max_entries"][..],
            other => {
                return Err(rpc_error(
                    METHOD_NOT_FOUND,
                    format!("unknown tool: {other}"),
                ));
            }
        };
        if let Err(error) = reject_unknown_arguments(&arguments, allowed) {
            return Ok(tool_result(Err(error)));
        }

        let outcome: anyhow::Result<Value> = match name {
            "wifi_status" => {
                wlan::wifi_status().map(|interfaces| json!({ "interfaces": interfaces }))
            }
            "wifi_scan" => wlan::bss::scan_and_wait(SCAN_TIMEOUT)
                .and_then(|refresh| Ok(serde_json::to_value(refresh)?)),
            "wifi_networks" => collect_networks(&arguments),
            "wifi_analyze" => analyze_environment(&arguments),
            "wifi_history" => history(&arguments),
            "wifi_sample" => sample(&arguments, context),
            "chronicle_start" => (|| -> anyhow::Result<Value> {
                let interval = bounded_u64(&arguments, "interval_seconds", 5, 1, 300)?;
                let threshold = bounded_i32(&arguments, "signal_threshold_db", 8, 1, 50)?;
                self.chronicle
                    .start(Duration::from_secs(interval), threshold)
            })(),
            "chronicle_stop" => self.chronicle.stop(),
            "chronicle_status" => Ok(self.chronicle.status()),
            "chronicle_recent" => (|| -> anyhow::Result<Value> {
                let max = bounded_u64(&arguments, "max_entries", 100, 1, 1000)? as usize;
                self.chronicle.recent(max)
            })(),
            _ => unreachable!("known tool names were validated above"),
        };

        Ok(tool_result(outcome))
    }

    fn read_resource(&self, params: &Value) -> Result<Value, RpcError> {
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
                self.chronicle.recent(100).and_then(|value| encode(&value)),
            ),
            other => {
                return Err(rpc_error(
                    RESOURCE_NOT_FOUND,
                    format!("resource not found: {other}"),
                ));
            }
        };
        let text = body.map_err(|error| rpc_error(INTERNAL_ERROR, error.to_string()))?;
        Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime, "text": text }] }))
    }
}

#[derive(Clone)]
struct RequestContext {
    cancelled: Arc<AtomicBool>,
    progress_token: Option<Value>,
    output: Option<mpsc::Sender<String>>,
}

impl RequestContext {
    fn idle() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            progress_token: None,
            output: None,
        }
    }

    fn check_cancelled(&self) -> anyhow::Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            anyhow::bail!("request cancelled by client");
        }
        Ok(())
    }

    fn progress(&self, progress: u128, total: u128, message: &str) {
        let (Some(token), Some(output)) = (&self.progress_token, &self.output) else {
            return;
        };
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": token,
                "progress": progress,
                "total": total,
                "message": message
            }
        })
        .to_string();
        let _ = output.send(notification);
    }
}

pub fn serve_stdio() -> anyhow::Result<()> {
    let server = Arc::new(Server::new());
    let (output_tx, output_rx) = mpsc::channel::<String>();
    let writer = std::thread::spawn(move || -> std::io::Result<()> {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        for frame in output_rx {
            writeln!(stdout, "{frame}")?;
            stdout.flush()?;
        }
        Ok(())
    });

    let stdin = std::io::stdin();
    let mut workers = Vec::new();
    let mut request_worker_panicked = false;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        // Joining completed workers releases their native thread handles and
        // keeps a long-lived MCP session from accumulating one handle per call.
        request_worker_panicked |= reap_finished_workers(&mut workers);

        if let Some((id, progress_token)) = tool_call_metadata(&line) {
            let server = server.clone();
            let response_tx = output_tx.clone();
            let progress_tx = output_tx.clone();
            let cancelled = server.register_request(&id);
            workers.push(std::thread::spawn(move || {
                let _registration = RegisteredRequest {
                    server: server.clone(),
                    id,
                };
                let context = RequestContext {
                    cancelled,
                    progress_token,
                    output: Some(progress_tx),
                };
                if let Some(response) = server.handle_line(&line, &context) {
                    let _ = response_tx.send(response);
                }
            }));
        } else if let Some(response) = server.handle_line(&line, &RequestContext::idle()) {
            output_tx
                .send(response)
                .map_err(|_| anyhow::anyhow!("stdout writer stopped"))?;
        }
    }

    for worker in workers {
        request_worker_panicked |= worker.join().is_err();
    }
    let _ = server.chronicle.stop();
    drop(output_tx);
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("stdout writer panicked"))??;
    if request_worker_panicked {
        anyhow::bail!("an MCP request worker panicked");
    }
    Ok(())
}

fn reap_finished_workers(workers: &mut Vec<std::thread::JoinHandle<()>>) -> bool {
    let mut panicked = false;
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            panicked |= worker.join().is_err();
        } else {
            index += 1;
        }
    }
    panicked
}

fn tool_call_metadata(line: &str) -> Option<(Value, Option<Value>)> {
    let value: Value = serde_json::from_str(line.trim_start_matches('\u{feff}')).ok()?;
    if value.get("method").and_then(Value::as_str) != Some("tools/call") {
        return None;
    }
    Some((
        value.get("id").cloned()?,
        value.pointer("/params/_meta/progressToken").cloned(),
    ))
}

fn request_key(id: &Value) -> String {
    serde_json::to_string(id).unwrap_or_else(|_| "null".to_string())
}

struct RpcError {
    code: i64,
    message: String,
}

fn rpc_error(code: i64, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
    }
}

fn tool_result(outcome: anyhow::Result<Value>) -> Value {
    match outcome {
        Ok(structured) => {
            let text = serde_json::to_string(&structured).unwrap_or_else(|error| error.to_string());
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": structured,
                "isError": false
            })
        }
        Err(error) => json!({
            "content": [{ "type": "text", "text": error.to_string() }],
            "isError": true
        }),
    }
}

fn tool_definitions() -> Value {
    let status_output = output_schema(
        &["interfaces"],
        json!({
            "interfaces": {"type":"array","items":{"type":"object"}}
        }),
    );
    let networks_output = output_schema(
        &["count", "detail", "interface_errors", "networks"],
        json!({
            "count":{"type":"integer","minimum":0},
            "detail":{"type":"string","enum":["summary","full"]},
            "cache_age_seconds":{"type":["integer","null"],"minimum":0},
            "refresh":{"type":["object","null"]},
            "interface_errors":{"type":"array","items":{"type":"object"}},
            "networks":{"type":"array","items":{"type":"object"}}
        }),
    );
    let analysis_output = output_schema(
        &["interface_errors", "analysis"],
        json!({
            "cache_age_seconds":{"type":["integer","null"],"minimum":0},
            "refresh":{"type":["object","null"]},
            "interface_errors":{"type":"array","items":{"type":"object"}},
            "analysis":{"type":"object"}
        }),
    );
    let history_output = output_schema(
        &["window_seconds", "event_count", "verdict", "events"],
        json!({
            "window_seconds":{"type":"integer","minimum":1},
            "event_count":{"type":"integer","minimum":0},
            "verdict":{"type":"object"},
            "events":{"type":["array","null"],"items":{"type":"object"}}
        }),
    );
    let sample_output = output_schema(
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
            "duration_s":{"type":"integer"},
            "interval_ms":{"type":"integer"},
            "sample_count":{"type":"integer"},
            "interface_guid":{"type":["string","null"]},
            "ssid":{"type":["string","null"]},
            "disconnected_samples":{"type":"integer"},
            "failed_samples":{"type":"integer"},
            "bssids_seen":{"type":"array","items":{"type":"string"}},
            "roam_count":{"type":"integer"},
            "rssi_min_dbm":{"type":["integer","null"]},
            "rssi_max_dbm":{"type":["integer","null"]},
            "rssi_mean_dbm":{"type":["number","null"]},
            "rssi_swing_db":{"type":["integer","null"]},
            "rx_rate_min_kbps":{"type":["integer","null"]},
            "rx_rate_max_kbps":{"type":["integer","null"]},
            "samples":{"type":"array","items":{"type":"object"}}
        }),
    );
    let scan_output = output_schema(
        &[
            "requested",
            "completed",
            "failed",
            "timed_out",
            "interfaces",
        ],
        json!({
            "requested":{"type":"integer"},
            "completed":{"type":"integer"},
            "failed":{"type":"integer"},
            "timed_out":{"type":"integer"},
            "elapsed_ms":{"type":"integer"},
            "observed_at_epoch_seconds":{"type":"integer"},
            "interfaces":{"type":"array","items":{"type":"object"}}
        }),
    );
    let chronicle_status_output = output_schema(
        &["running", "path"],
        json!({
            "running":{"type":"boolean"},
            "path":{"type":"string"},
            "started_at_epoch_seconds":{"type":["integer","null"]},
            "stopped_at_epoch_seconds":{"type":["integer","null"]},
            "entries_written_this_run":{"type":"integer"},
            "last_error":{"type":["string","null"]}
        }),
    );
    let chronicle_recent_output = output_schema(
        &["path", "count", "entries"],
        json!({
            "path":{"type":"string"},
            "count":{"type":"integer"},
            "invalid_lines":{"type":"integer"},
            "entries":{"type":"array","items":{"type":"object"}}
        }),
    );
    json!([
        tool("wifi_status", "Wi-Fi status", "Current state of every WLAN interface.", json!({"type":"object","properties":{},"additionalProperties":false}), status_output, true, true),
        tool("wifi_networks", "Visible Wi-Fi networks", "Nearby BSS records with real dBm, security, channel width and load. refresh_scan initiates a standard radio scan.", json!({"type":"object","properties":{"refresh_scan":{"type":"boolean"},"detail":{"type":"string","enum":["summary","full"]}},"additionalProperties":false}), networks_output, false, true),
        tool("wifi_analyze", "Analyze Wi-Fi environment", "Caveated findings for signal, contention, roaming candidates and security.", json!({"type":"object","properties":{"refresh_scan":{"type":"boolean"}},"additionalProperties":false}), analysis_output, false, true),
        tool("wifi_history", "Wi-Fi event history", "Windows WLAN AutoConfig history and evidence-based verdicts.", json!({"type":"object","properties":{"within_seconds":{"type":"integer","minimum":1,"maximum":MAX_HISTORY_WINDOW_S},"max_events":{"type":"integer","minimum":1,"maximum":2000},"include_events":{"type":"boolean"}},"additionalProperties":false}), history_output, true, true),
        tool("wifi_sample", "Sample Wi-Fi connection", "Cancelable sampling with RSSI/rate/roaming aggregates and optional interface selection.", json!({"type":"object","properties":{"interface_guid":{"type":"string"},"duration_seconds":{"type":"integer","minimum":1,"maximum":120},"interval_ms":{"type":"integer","minimum":250,"maximum":60000}},"additionalProperties":false}), sample_output, true, false),
        tool("wifi_scan", "Refresh Wi-Fi scan", "Initiate a standard Wi-Fi scan and wait for per-interface completion notifications.", json!({"type":"object","properties":{},"additionalProperties":false}), scan_output, false, false),
        tool("chronicle_start", "Start RadioChron chronicle", "Start the local change-only JSONL recorder in LocalAppData.", json!({"type":"object","properties":{"interval_seconds":{"type":"integer","minimum":1,"maximum":300},"signal_threshold_db":{"type":"integer","minimum":1,"maximum":50}},"additionalProperties":false}), chronicle_status_output.clone(), false, false),
        tool("chronicle_stop", "Stop RadioChron chronicle", "Stop and flush the process-local recorder.", json!({"type":"object","properties":{},"additionalProperties":false}), chronicle_status_output.clone(), false, false),
        tool("chronicle_status", "Chronicle status", "Read recorder state and storage path.", json!({"type":"object","properties":{},"additionalProperties":false}), chronicle_status_output, true, true),
        tool("chronicle_recent", "Recent chronicle changes", "Read recent change-only entries across the active and rotated JSONL files.", json!({"type":"object","properties":{"max_entries":{"type":"integer","minimum":1,"maximum":1000}},"additionalProperties":false}), chronicle_recent_output, true, true)
    ])
}

fn output_schema(required: &[&str], properties: Value) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    output_schema: Value,
    read_only: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "outputSchema": output_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
            "openWorldHint": false
        }
    })
}

fn resource_definitions() -> Value {
    json!([
        {"uri":URI_REPORT_MD,"name":"wifi_diagnostic_report","title":"Wi-Fi Diagnostic Report","description":"Current cached diagnostic report.","mimeType":"text/markdown","annotations":{"audience":["user","assistant"],"priority":0.9}},
        {"uri":URI_REPORT_JSON,"name":"wifi_diagnostic_report_json","title":"Wi-Fi Diagnostic Report (JSON)","description":"Machine-readable cached report.","mimeType":"application/json"},
        {"uri":URI_STATUS,"name":"wifi_status","title":"Current Wi-Fi Connection","description":"Per-interface association state.","mimeType":"application/json"},
        {"uri":URI_NETWORKS,"name":"wifi_networks","title":"Visible Networks","description":"Compact cached BSS list.","mimeType":"application/json"},
        {"uri":URI_CHRONICLE,"name":"chronicle_recent","title":"Recent Radio Changes","description":"Recent entries from the local change-only chronicle.","mimeType":"application/json"}
    ])
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
        Format::Markdown | Format::Json => {
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
                _ => {
                    let mut report = crate::report::json(&status, entries, &analysis);
                    if let Some(object) = report.as_object_mut() {
                        object.insert(
                            "scan_interface_errors".to_string(),
                            serde_json::to_value(collection.interface_errors)?,
                        );
                    }
                    encode(&report)
                }
            }
        }
    }
}

fn collect_networks(arguments: &Value) -> anyhow::Result<Value> {
    let detail = optional_string(arguments, "detail")?.unwrap_or("summary");
    if !matches!(detail, "summary" | "full") {
        anyhow::bail!("detail must be summary or full");
    }
    let full = detail == "full";
    let requested_refresh = optional_bool(arguments, "refresh_scan", false)?;
    let mut refresh = if requested_refresh {
        Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?)
    } else {
        None
    };
    let mut collection = wlan::bss::bss_list_detailed()?;
    if collection.entries.is_empty() && refresh.is_none() {
        refresh = Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?);
        collection = wlan::bss::bss_list_detailed()?;
    }
    let networks = if full {
        serde_json::to_value(&collection.entries)?
    } else {
        serde_json::to_value(
            collection
                .entries
                .iter()
                .map(BssSummary::from)
                .collect::<Vec<_>>(),
        )?
    };
    Ok(json!({
        "count": collection.entries.len(),
        "detail": if full { "full" } else { "summary" },
        "cache_age_seconds": wlan::bss::last_refresh_age_seconds(),
        "refresh": refresh,
        "interface_errors": collection.interface_errors,
        "networks": networks
    }))
}

fn analyze_environment(arguments: &Value) -> anyhow::Result<Value> {
    let refresh = if optional_bool(arguments, "refresh_scan", false)? {
        Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?)
    } else {
        None
    };
    let collection = wlan::bss::bss_list_detailed()?;
    let status = wlan::wifi_status()?;
    let connection = status.iter().find_map(|entry| entry.connection.as_ref());
    Ok(json!({
        "cache_age_seconds": wlan::bss::last_refresh_age_seconds(),
        "refresh": refresh,
        "interface_errors": collection.interface_errors,
        "analysis": wlan::analyze::analyze(&collection.entries, connection)
    }))
}

fn history(arguments: &Value) -> anyhow::Result<Value> {
    let within = bounded_u64(arguments, "within_seconds", 3600, 1, MAX_HISTORY_WINDOW_S)?;
    let max = bounded_u64(arguments, "max_events", 200, 1, 2000)? as usize;
    let events = radiochron::events::recent(max, Some(within))?;
    let verdict = radiochron::events::detect(&events);
    let include = optional_bool(arguments, "include_events", false)?;
    Ok(json!({
        "window_seconds": within,
        "event_count": events.len(),
        "verdict": verdict,
        "events": if include { serde_json::to_value(&events)? } else { Value::Null }
    }))
}

fn sample(arguments: &Value, context: &RequestContext) -> anyhow::Result<Value> {
    let duration = bounded_u64(arguments, "duration_seconds", 20, 1, 120)?;
    let interval = bounded_u64(arguments, "interval_ms", 1000, 250, 60_000)?;
    let interface = optional_string(arguments, "interface_guid")?;
    context.check_cancelled()?;
    let run =
        wlan::sample::sample_connection_on_controlled(interface, duration, interval, |progress| {
            context.check_cancelled()?;
            context.progress(
                progress.elapsed_ms,
                progress.total_ms,
                &format!("{} samples collected", progress.sample_count),
            );
            Ok(())
        })?;
    Ok(serde_json::to_value(run)?)
}

fn bounded_u64(
    arguments: &Value,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> anyhow::Result<u64> {
    Ok(bounded_integer(arguments, name, default as i64, min as i64, max as i64)? as u64)
}

fn bounded_i32(
    arguments: &Value,
    name: &str,
    default: i32,
    min: i32,
    max: i32,
) -> anyhow::Result<i32> {
    Ok(bounded_integer(
        arguments,
        name,
        i64::from(default),
        i64::from(min),
        i64::from(max),
    )? as i32)
}

fn bounded_integer(
    arguments: &Value,
    name: &str,
    default: i64,
    min: i64,
    max: i64,
) -> anyhow::Result<i64> {
    let Some(value) = arguments.get(name) else {
        return Ok(default);
    };
    let value = value
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("{name} must be an integer"))?;
    if !(min..=max).contains(&value) {
        anyhow::bail!("{name} must be between {min} and {max}");
    }
    Ok(value)
}

fn reject_unknown_arguments(arguments: &Value, allowed: &[&str]) -> anyhow::Result<()> {
    let object = arguments
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("tool arguments must be an object"))?;
    if let Some(name) = object.keys().find(|name| !allowed.contains(&name.as_str())) {
        anyhow::bail!("unknown argument: {name}");
    }
    Ok(())
}

fn optional_bool(arguments: &Value, name: &str, default: bool) -> anyhow::Result<bool> {
    match arguments.get(name) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("{name} must be a boolean")),
    }
}

fn optional_string<'a>(arguments: &'a Value, name: &str) -> anyhow::Result<Option<&'a str>> {
    match arguments.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{name} must be a string")),
    }
}

fn encode<T: Serialize>(value: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_server() -> Server {
        let server = Server::new();
        let initialize = server
            .handle_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","clientInfo":{"name":"test","version":"1"},"capabilities":{}}}"#,
                &RequestContext::idle(),
            )
            .unwrap();
        let initialize: Value = serde_json::from_str(&initialize).unwrap();
        assert_eq!(initialize["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(server
            .handle_line(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &RequestContext::idle()
            )
            .is_none());
        server
    }

    fn response(server: &Server, line: &str) -> Value {
        serde_json::from_str(
            &server
                .handle_line(line, &RequestContext::idle())
                .expect("expected response"),
        )
        .unwrap()
    }

    #[test]
    fn lifecycle_is_required_and_version_is_negotiated() {
        let server = Server::new();
        let early = response(&server, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert_eq!(early["error"]["code"], INVALID_REQUEST);
        let _ = ready_server();
    }

    #[test]
    fn invalid_jsonrpc_version_is_rejected() {
        let server = Server::new();
        let out = response(
            &server,
            r#"{"jsonrpc":"1.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        );
        assert_eq!(out["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn tools_have_structured_schemas_and_truthful_annotations() {
        let server = ready_server();
        let out = response(&server, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = out["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 10);
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(tool["outputSchema"]["type"], "object");
            assert!(tool["annotations"]["destructiveHint"] == false);
        }
        let scan = tools
            .iter()
            .find(|tool| tool["name"] == "wifi_scan")
            .unwrap();
        assert_eq!(scan["annotations"]["readOnlyHint"], false);
        let status = tools
            .iter()
            .find(|tool| tool["name"] == "wifi_status")
            .unwrap();
        assert_eq!(status["annotations"]["readOnlyHint"], true);

        let sample = tools
            .iter()
            .find(|tool| tool["name"] == "wifi_sample")
            .unwrap();
        assert!(sample["outputSchema"]["properties"]["ssid"].is_object());
        assert!(sample["outputSchema"]["properties"]["rssi_mean_dbm"].is_object());
        assert!(scan["outputSchema"]["properties"]["observed_at_epoch_seconds"].is_object());
    }

    #[test]
    fn resources_include_the_chronicle() {
        let server = ready_server();
        let out = response(
            &server,
            r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#,
        );
        let resources = out["result"]["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 5);
        assert!(resources
            .iter()
            .any(|resource| resource["uri"] == URI_CHRONICLE));
    }

    #[test]
    fn notifications_get_no_reply_and_can_cancel_registered_work() {
        let server = ready_server();
        let flag = server.register_request(&json!(42));
        assert!(server
            .handle_line(
                r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":42}}"#,
                &RequestContext::idle()
            )
            .is_none());
        assert!(flag.load(Ordering::Acquire));
    }

    #[test]
    fn malformed_json_reports_a_parse_error() {
        let server = Server::new();
        let out = response(&server, "{ not json");
        assert_eq!(out["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn unknown_resource_has_the_dedicated_error() {
        let server = ready_server();
        let out = response(
            &server,
            r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"radiochron://bogus"}}"#,
        );
        assert_eq!(out["error"]["code"], RESOURCE_NOT_FOUND);
    }

    #[test]
    fn tool_result_contains_text_and_structured_content() {
        let value = tool_result(Ok(json!({"ok":true})));
        assert_eq!(value["structuredContent"]["ok"], true);
        assert_eq!(value["isError"], false);
        assert!(value["content"][0]["text"].is_string());
    }

    #[test]
    fn numeric_arguments_are_rejected_not_silently_clamped() {
        assert!(bounded_u64(&json!({"n": 0}), "n", 5, 1, 10).is_err());
        assert!(bounded_u64(&json!({"n": -1}), "n", 5, 1, 10).is_err());
        assert_eq!(bounded_u64(&json!({}), "n", 5, 1, 10).unwrap(), 5);
        assert!(optional_bool(&json!({"refresh": "yes"}), "refresh", false).is_err());
        assert!(reject_unknown_arguments(&json!({"surprise": true}), &[]).is_err());
    }
}
