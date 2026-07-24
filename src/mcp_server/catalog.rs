mod chronicle;
mod incident;
mod wifi;

use serde_json::{json, Value};

use super::LATEST_PROTOCOL_VERSION;

pub(super) fn tool_definitions(protocol_version: &str) -> Value {
    let mut tools = wifi::definitions(protocol_version);
    tools.extend(chronicle::definitions(protocol_version));
    tools.extend(crate::ble::tool_definitions(protocol_version));
    tools.push(incident::definition(protocol_version));
    json!(tools)
}

pub(crate) fn output_schema(required: &[&str], properties: Value) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn tool(
    protocol_version: &str,
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    output_schema: Value,
    read_only: bool,
    idempotent: bool,
    open_world: bool,
) -> Value {
    let mut definition = json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "outputSchema": output_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": idempotent,
            "openWorldHint": open_world
        }
    });
    if protocol_version == LATEST_PROTOCOL_VERSION {
        definition["execution"] = json!({"taskSupport": "forbidden"});
    }
    definition
}
