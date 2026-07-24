mod validation;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::schema::{error_response, rpc_error, rpc_error_with_data, success_response, RpcError};
use super::transport::RequestContext;
use super::{
    catalog, resources, INVALID_PARAMS, INVALID_REQUEST, LATEST_PROTOCOL_VERSION,
    LEGACY_PROTOCOL_VERSION, METHOD_NOT_FOUND, PARSE_ERROR, SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::{ble::BleService, chronicle::ChronicleService};
use validation::{
    require_notification, require_object_field, require_string_field, valid_request_id,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Uninitialized,
    Initializing,
    Ready,
}

#[derive(Debug)]
struct Lifecycle {
    phase: Phase,
    protocol_version: Option<&'static str>,
}

pub(super) struct Server {
    lifecycle: Mutex<Lifecycle>,
    pub(super) ble: BleService,
    pub(super) chronicle: ChronicleService,
    cancellations: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

pub(super) struct RegisteredRequest {
    pub(super) server: Arc<Server>,
    pub(super) id: Value,
}

impl Drop for RegisteredRequest {
    fn drop(&mut self) {
        self.server.finish_request(&self.id);
    }
}

impl Server {
    pub(super) fn new() -> Self {
        Self {
            lifecycle: Mutex::new(Lifecycle {
                phase: Phase::Uninitialized,
                protocol_version: None,
            }),
            ble: BleService::new(),
            chronicle: ChronicleService::new(),
            cancellations: Mutex::new(HashMap::new()),
        }
    }

    pub(super) fn handle_line(&self, line: &str, context: &RequestContext) -> Option<String> {
        let line = line.trim_start_matches('\u{feff}');
        let message: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                return Some(error_response(
                    Value::Null,
                    rpc_error_with_data(
                        PARSE_ERROR,
                        "Parse error",
                        json!({"detail": error.to_string()}),
                    ),
                ));
            }
        };
        let Some(object) = message.as_object() else {
            return Some(error_response(
                Value::Null,
                rpc_error(INVALID_REQUEST, "request must be a JSON-RPC 2.0 object"),
            ));
        };
        if object.get("jsonrpc") != Some(&Value::String("2.0".into())) {
            return Some(error_response(
                Value::Null,
                rpc_error(INVALID_REQUEST, "jsonrpc must equal 2.0"),
            ));
        }

        let id = match object.get("id") {
            Some(value) if !valid_request_id(value) => {
                return Some(error_response(
                    Value::Null,
                    rpc_error(INVALID_REQUEST, "id must be a string or integer"),
                ));
            }
            value => value.cloned(),
        };
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(error_response(
                id.unwrap_or(Value::Null),
                rpc_error(INVALID_REQUEST, "missing method"),
            ));
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        if !params.is_null() && !params.is_object() {
            return id.map(|id| {
                error_response(
                    id,
                    rpc_error(INVALID_PARAMS, "params must be an object when present"),
                )
            });
        }

        let is_notification = id.is_none();
        if is_notification && method == "initialize" {
            return None;
        }
        match self.dispatch(method, &params, context, is_notification) {
            Ok(result) => id.map(|id| success_response(id, result)),
            Err(error) => id.map(|id| error_response(id, error)),
        }
    }

    fn dispatch(
        &self,
        method: &str,
        params: &Value,
        context: &RequestContext,
        is_notification: bool,
    ) -> Result<Value, RpcError> {
        match method {
            "initialize" => return self.initialize(params),
            "notifications/initialized" => {
                require_notification(is_notification, method)?;
                return self.mark_ready();
            }
            "notifications/cancelled" => {
                require_notification(is_notification, method)?;
                self.cancel(params)?;
                return Ok(json!({}));
            }
            "ping" => return Ok(json!({})),
            _ => {}
        }

        let protocol_version = self.ready_protocol_version()?;
        match method {
            "tools/list" => Ok(json!({
                "tools": catalog::tool_definitions(protocol_version)
            })),
            "tools/call" => super::tools::call(self, params, context),
            "resources/list" => Ok(json!({ "resources": resources::definitions() })),
            "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
            "resources/read" => resources::read(self, params),
            other => Err(rpc_error(
                METHOD_NOT_FOUND,
                format!("unknown method: {other}"),
            )),
        }
    }

    fn initialize(&self, params: &Value) -> Result<Value, RpcError> {
        let object = params.as_object().ok_or_else(|| {
            rpc_error(
                INVALID_PARAMS,
                "initialize requires protocolVersion, capabilities, and clientInfo",
            )
        })?;
        let requested = object
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(INVALID_PARAMS, "initialize requires protocolVersion"))?;
        require_object_field(object, "capabilities")?;
        let client_info = require_object_field(object, "clientInfo")?;
        require_string_field(client_info, "name")?;
        require_string_field(client_info, "version")?;

        let negotiated = SUPPORTED_PROTOCOL_VERSIONS
            .iter()
            .copied()
            .find(|version| *version == requested)
            .unwrap_or(LATEST_PROTOCOL_VERSION);
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if lifecycle.phase != Phase::Uninitialized {
            return Err(rpc_error(INVALID_REQUEST, "server is already initialized"));
        }
        lifecycle.phase = Phase::Initializing;
        lifecycle.protocol_version = Some(negotiated);

        let server_info = if negotiated == LEGACY_PROTOCOL_VERSION {
            json!({
                "name": "radiochron",
                "title": "RadioChron local radio diagnostics",
                "version": env!("CARGO_PKG_VERSION")
            })
        } else {
            json!({
                "name": "radiochron",
                "title": "RadioChron local radio diagnostics",
                "version": env!("CARGO_PKG_VERSION"),
                "description": "Local Wi-Fi incident diagnostics and native BLE observation/history.",
                "websiteUrl": "https://radiochron.com"
            })
        };
        Ok(json!({
            "protocolVersion": negotiated,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false }
            },
            "serverInfo": server_info,
            "instructions": "Use diagnose_incident for a compact Wi-Fi/connectivity/history/chronicle/BLE snapshot. Native BLE scanning never connects to devices. SSIDs, BSSIDs and radio identifiers are sensitive. RSSI is not physical distance; findings include evidence limitations."
        }))
    }

    fn mark_ready(&self) -> Result<Value, RpcError> {
        let mut lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if lifecycle.phase != Phase::Initializing {
            return Err(rpc_error(INVALID_REQUEST, "server was not initializing"));
        }
        lifecycle.phase = Phase::Ready;
        Ok(json!({}))
    }

    fn ready_protocol_version(&self) -> Result<&'static str, RpcError> {
        let lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if lifecycle.phase != Phase::Ready {
            return Err(rpc_error(
                INVALID_REQUEST,
                "initialize and notifications/initialized must complete first",
            ));
        }
        Ok(lifecycle
            .protocol_version
            .unwrap_or(LATEST_PROTOCOL_VERSION))
    }

    fn cancel(&self, params: &Value) -> Result<(), RpcError> {
        let request_id = params
            .get("requestId")
            .filter(|value| valid_request_id(value))
            .ok_or_else(|| {
                rpc_error(INVALID_PARAMS, "notifications/cancelled requires requestId")
            })?;
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

    pub(super) fn register_request(&self, id: &Value) -> Arc<AtomicBool> {
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
}

fn request_key(id: &Value) -> String {
    id.to_string()
}
