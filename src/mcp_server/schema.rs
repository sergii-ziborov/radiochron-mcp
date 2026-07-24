use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};

#[derive(Debug)]
pub(super) struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

pub(super) fn rpc_error(code: i64, message: impl Into<String>) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: None,
    }
}

pub(super) fn rpc_error_with_data(code: i64, message: impl Into<String>, data: Value) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: Some(data),
    }
}

pub(super) fn tool_result(outcome: anyhow::Result<Value>) -> Value {
    match outcome {
        Ok(value) => json!({
            "content": [{"type": "text", "text": value.to_string()}],
            "structuredContent": value,
            "isError": false
        }),
        Err(error) => json!({
            "content": [{"type": "text", "text": error.to_string()}],
            "isError": true
        }),
    }
}

pub(super) fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

pub(super) fn error_response(id: Value, error: RpcError) -> String {
    let mut body = Map::from_iter([
        ("code".to_string(), json!(error.code)),
        ("message".to_string(), json!(error.message)),
    ]);
    if let Some(data) = error.data {
        body.insert("data".to_string(), data);
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": body }).to_string()
}

pub(super) fn encode<T: Serialize>(value: &T) -> anyhow::Result<String> {
    Ok(serde_json::to_string(value)?)
}

pub(crate) fn required<T: DeserializeOwned>(arguments: &Value, name: &str) -> anyhow::Result<T> {
    let value = arguments
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))?;
    Ok(serde_json::from_value(value.clone())?)
}

pub(super) fn reject_unknown_arguments(arguments: &Value, allowed: &[&str]) -> anyhow::Result<()> {
    let object = arguments
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("tool arguments must be an object"))?;
    if let Some(name) = object.keys().find(|name| !allowed.contains(&name.as_str())) {
        anyhow::bail!("unknown argument: {name}");
    }
    Ok(())
}

pub(crate) fn optional_bool(arguments: &Value, name: &str, default: bool) -> anyhow::Result<bool> {
    match arguments.get(name) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("{name} must be a boolean")),
    }
}

pub(super) fn optional_string<'a>(
    arguments: &'a Value,
    name: &str,
) -> anyhow::Result<Option<&'a str>> {
    match arguments.get(name) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("{name} must be a string")),
    }
}

pub(crate) fn bounded_optional_string(
    arguments: &Value,
    name: &str,
    max_len: usize,
) -> anyhow::Result<Option<String>> {
    let Some(value) = optional_string(arguments, name)? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.len() > max_len {
        anyhow::bail!("{name} must contain 1..={max_len} bytes");
    }
    Ok(Some(value.to_string()))
}

pub(crate) fn bounded_u64(
    arguments: &Value,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> anyhow::Result<u64> {
    Ok(bounded_integer(arguments, name, default as i64, min as i64, max as i64)? as u64)
}

pub(super) fn bounded_i32(
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
