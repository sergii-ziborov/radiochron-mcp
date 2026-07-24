use serde_json::{Map, Value};

use super::super::schema::{rpc_error, RpcError};
use super::super::{INVALID_PARAMS, INVALID_REQUEST};

pub(super) fn valid_request_id(value: &Value) -> bool {
    value.is_string() || value.as_i64().is_some() || value.as_u64().is_some()
}

pub(super) fn require_notification(is_notification: bool, method: &str) -> Result<(), RpcError> {
    if is_notification {
        Ok(())
    } else {
        Err(rpc_error(
            INVALID_REQUEST,
            format!("{method} must be a notification"),
        ))
    }
}

pub(super) fn require_object_field<'a>(
    object: &'a Map<String, Value>,
    name: &str,
) -> Result<&'a Map<String, Value>, RpcError> {
    object
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, format!("initialize requires object {name}")))
}

pub(super) fn require_string_field(
    object: &Map<String, Value>,
    name: &str,
) -> Result<(), RpcError> {
    if object.get(name).and_then(Value::as_str).is_some() {
        Ok(())
    } else {
        Err(rpc_error(
            INVALID_PARAMS,
            format!("clientInfo requires string {name}"),
        ))
    }
}
