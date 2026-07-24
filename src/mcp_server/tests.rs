use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use super::protocol::Server;
use super::resources::{URI_BLE_HISTORIES, URI_CHRONICLE};
use super::schema::{bounded_u64, optional_bool, reject_unknown_arguments, tool_result};
use super::transport::RequestContext;
use super::{
    INVALID_PARAMS, INVALID_REQUEST, LATEST_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION, PARSE_ERROR,
    RESOURCE_NOT_FOUND,
};

fn ready_server(version: &str) -> Server {
    let server = Server::new();
    let initialize = response(
        &server,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","clientInfo":{{"name":"test","version":"1"}},"capabilities":{{}}}}}}"#
        ),
    );
    assert_eq!(initialize["result"]["protocolVersion"], version);
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
fn lifecycle_negotiates_current_legacy_and_unknown_versions() {
    let server = Server::new();
    let early = response(&server, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert_eq!(early["error"]["code"], INVALID_REQUEST);
    let _ = ready_server(LATEST_PROTOCOL_VERSION);
    let _ = ready_server(LEGACY_PROTOCOL_VERSION);

    let current_info = initialize_response(LATEST_PROTOCOL_VERSION);
    assert!(current_info["result"]["serverInfo"]["description"].is_string());
    assert!(current_info["result"]["serverInfo"]["websiteUrl"].is_string());
    let legacy_info = initialize_response(LEGACY_PROTOCOL_VERSION);
    assert!(legacy_info["result"]["serverInfo"]
        .get("description")
        .is_none());
    assert!(legacy_info["result"]["serverInfo"]
        .get("websiteUrl")
        .is_none());

    let server = Server::new();
    let out = response(
        &server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01","clientInfo":{"name":"test","version":"1"},"capabilities":{}}}"#,
    );
    assert_eq!(out["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
}

fn initialize_response(version: &str) -> Value {
    let server = Server::new();
    response(
        &server,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}","clientInfo":{{"name":"test","version":"1"}},"capabilities":{{}}}}}}"#
        ),
    )
}

#[test]
fn initialize_requires_client_contract_without_poisoning_lifecycle() {
    let server = Server::new();
    let invalid = response(
        &server,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#,
    );
    assert_eq!(invalid["error"]["code"], INVALID_PARAMS);
    let valid = response(
        &server,
        r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}}"#,
    );
    assert_eq!(valid["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
}

#[test]
fn malformed_envelopes_use_json_rpc_errors() {
    let server = Server::new();
    assert_eq!(
        response(&server, "{ not json")["error"]["code"],
        PARSE_ERROR
    );
    assert_eq!(
        response(
            &server,
            r#"{"jsonrpc":"1.0","id":1,"method":"initialize","params":{}}"#
        )["error"]["code"],
        INVALID_REQUEST
    );
    assert_eq!(
        response(
            &server,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"initialize","params":{}}"#
        )["error"]["code"],
        INVALID_REQUEST
    );
}

#[test]
fn tool_catalog_is_versioned_and_complete() {
    let current = ready_server(LATEST_PROTOCOL_VERSION);
    let out = response(
        &current,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let tools = out["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), if cfg!(windows) { 18 } else { 17 });
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["outputSchema"]["type"], "object");
        assert_eq!(tool["annotations"]["destructiveHint"], false);
        assert_eq!(tool["execution"]["taskSupport"], "forbidden");
    }
    assert!(tools.iter().any(|tool| tool["name"] == "ble_scan"));
    assert!(tools.iter().any(|tool| tool["name"] == "diagnose_incident"));

    let legacy = ready_server(LEGACY_PROTOCOL_VERSION);
    let out = response(&legacy, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    assert!(out["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool.get("execution").is_none()));
}

#[test]
fn unknown_tools_are_protocol_errors_and_bad_inputs_are_tool_errors() {
    let server = ready_server(LATEST_PROTOCOL_VERSION);
    let unknown = response(
        &server,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"missing","arguments":{}}}"#,
    );
    assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
    let invalid = response(
        &server,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wifi_status","arguments":{"surprise":true}}}"#,
    );
    assert_eq!(invalid["result"]["isError"], true);

    let invalid_incident = response(
        &server,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"diagnose_incident","arguments":{"include_ble":false,"history_max_events":"many"}}}"#,
    );
    assert_eq!(invalid_incident["result"]["isError"], true);
}

#[test]
fn resources_include_chronicle_and_native_ble_history() {
    let server = ready_server(LATEST_PROTOCOL_VERSION);
    let out = response(
        &server,
        r#"{"jsonrpc":"2.0","id":3,"method":"resources/list"}"#,
    );
    let resources = out["result"]["resources"].as_array().unwrap();
    assert_eq!(resources.len(), 6);
    assert!(resources
        .iter()
        .any(|resource| resource["uri"] == URI_CHRONICLE));
    assert!(resources
        .iter()
        .any(|resource| resource["uri"] == URI_BLE_HISTORIES));

    let missing = response(
        &server,
        r#"{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"radiochron://bogus"}}"#,
    );
    assert_eq!(missing["error"]["code"], RESOURCE_NOT_FOUND);
}

#[test]
fn notifications_do_not_reply_and_cancellation_reaches_workers() {
    let server = ready_server(LATEST_PROTOCOL_VERSION);
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
fn helpers_preserve_structured_results_and_reject_bad_numbers() {
    let value = tool_result(Ok(json!({"ok":true})));
    assert_eq!(value["structuredContent"]["ok"], true);
    assert_eq!(value["isError"], false);
    assert!(value["content"][0]["text"].is_string());
    assert!(bounded_u64(&json!({"n": 0}), "n", 5, 1, 10).is_err());
    assert!(bounded_u64(&json!({"n": -1}), "n", 5, 1, 10).is_err());
    assert_eq!(bounded_u64(&json!({}), "n", 5, 1, 10).unwrap(), 5);
    assert!(optional_bool(&json!({"refresh":"yes"}), "refresh", false).is_err());
    assert!(reject_unknown_arguments(&json!({"surprise":true}), &[]).is_err());
}

#[test]
fn rust_source_files_stay_below_the_architecture_limit() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut pending = vec![src];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                let line_count = std::fs::read_to_string(&path).unwrap().lines().count();
                assert!(
                    line_count <= 300,
                    "{} has {line_count} lines; limit is 300",
                    path.display()
                );
            }
        }
    }
}
