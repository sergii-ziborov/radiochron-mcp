use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_radiochron"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start MCP server");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn request(&mut self, value: Value) -> Value {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    fn notify(&mut self, value: Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn current_stdio_lifecycle_and_catalog_conform() {
    let mut session = Session::start();
    let initialized = session.request(json!({
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"conformance-test","version":"1"}
        }
    }));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        initialized["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    session.notify(json!({
        "jsonrpc":"2.0","method":"notifications/initialized"
    }));
    let tools = session.request(json!({
        "jsonrpc":"2.0","id":2,"method":"tools/list","params":{}
    }));
    let tools = tools["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "ble_scan"));
    assert!(tools.iter().any(|tool| tool["name"] == "diagnose_incident"));
    assert!(tools
        .iter()
        .all(|tool| tool["execution"]["taskSupport"] == "forbidden"));
}

#[test]
fn legacy_stdio_client_receives_legacy_catalog_shape() {
    let mut session = Session::start();
    let initialized = session.request(json!({
        "jsonrpc":"2.0","id":"init","method":"initialize",
        "params":{
            "protocolVersion":"2025-06-18",
            "capabilities":{},
            "clientInfo":{"name":"legacy-test","version":"1"}
        }
    }));
    assert_eq!(initialized["result"]["protocolVersion"], "2025-06-18");
    session.notify(json!({
        "jsonrpc":"2.0","method":"notifications/initialized"
    }));
    let tools = session.request(json!({
        "jsonrpc":"2.0","id":2,"method":"tools/list"
    }));
    assert!(tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .all(|tool| tool.get("execution").is_none()));
}

#[test]
fn execution_and_protocol_errors_use_distinct_channels() {
    let mut session = Session::start();
    let _ = session.request(json!({
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"error-test","version":"1"}
        }
    }));
    session.notify(json!({
        "jsonrpc":"2.0","method":"notifications/initialized"
    }));
    let unknown = session.request(json!({
        "jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"not_a_tool","arguments":{}}
    }));
    assert_eq!(unknown["error"]["code"], -32602);
    let invalid = session.request(json!({
        "jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"wifi_status","arguments":{"unexpected":true}}
    }));
    assert_eq!(invalid["result"]["isError"], true);
}
