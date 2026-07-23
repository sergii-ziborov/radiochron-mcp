use std::sync::{Mutex, MutexGuard};

use radiochron::ble::{Advertisement, Observation, Tracker, TrackerPolicy};
use serde_json::{json, Value};

pub struct BleService {
    tracker: Mutex<Tracker>,
}

impl BleService {
    pub fn new() -> Self {
        Self {
            tracker: Mutex::new(Tracker::new(TrackerPolicy::default())),
        }
    }

    pub fn identify(&self, arguments: &Value) -> anyhow::Result<Value> {
        let advertisement: Advertisement = required(arguments, "advertisement")?;
        Ok(json!({
            "identity": radiochron::ble::identify(&advertisement),
            "payload_hash": radiochron::ble::payload_hash(&advertisement)
        }))
    }

    pub fn reset(&self, arguments: &Value) -> anyhow::Result<Value> {
        let policy = arguments
            .get("policy")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();
        *self.lock()? = Tracker::new(policy);
        Ok(json!({"reset": true}))
    }

    pub fn observe(&self, arguments: &Value) -> anyhow::Result<Value> {
        let observation: Observation = required(arguments, "observation")?;
        Ok(serde_json::to_value(self.lock()?.observe(observation))?)
    }

    pub fn histories(&self) -> anyhow::Result<Value> {
        let tracker = self.lock()?;
        Ok(json!({
            "histories": tracker.histories().cloned().collect::<Vec<_>>()
        }))
    }

    pub fn evaluate(&self, arguments: &Value) -> anyhow::Result<Value> {
        let now_ms = arguments
            .get("now_ms")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("now_ms must be a non-negative integer"))?;
        Ok(json!({"findings": self.lock()?.evaluate(now_ms)}))
    }

    fn lock(&self) -> anyhow::Result<MutexGuard<'_, Tracker>> {
        self.tracker
            .lock()
            .map_err(|_| anyhow::anyhow!("BLE tracker lock poisoned"))
    }
}

pub fn tool_definitions() -> Vec<Value> {
    vec![
        definition(
            "ble_identify",
            "Identify BLE advertisement",
            "Derive a privacy-minimized identity and payload hash from a caller-supplied BLE advertisement. This tool does not start a scanner.",
            json!({"type":"object","properties":{"advertisement":advertisement_schema()},"required":["advertisement"],"additionalProperties":false}),
            json!({"type":"object","properties":{"identity":{"type":"object"},"payload_hash":{"type":"string"}},"required":["identity","payload_hash"],"additionalProperties":false}),
            true,
            true,
        ),
        definition(
            "ble_tracker_reset",
            "Reset BLE tracker",
            "Clear process-local BLE history and optionally apply detector thresholds, allowlist and expected identities.",
            json!({"type":"object","properties":{"policy":{"type":"object"}},"additionalProperties":false}),
            json!({"type":"object","properties":{"reset":{"const":true}},"required":["reset"],"additionalProperties":false}),
            false,
            true,
        ),
        definition(
            "ble_observe",
            "Observe BLE advertisement",
            "Add one caller-supplied timed BLE observation to process-local history and return evidence-based findings. RSSI is not interpreted as distance.",
            json!({"type":"object","properties":{"observation":{"type":"object"}},"required":["observation"],"additionalProperties":false}),
            json!({"type":"object","properties":{"identity":{"type":"object"},"payload_hash":{"type":"string"},"history":{"type":"object"},"findings":{"type":"array","items":{"type":"object"}}},"required":["identity","payload_hash","history","findings"],"additionalProperties":false}),
            false,
            false,
        ),
        definition(
            "ble_histories",
            "BLE histories",
            "Read process-local first/last seen, recurrence, sensor and RSSI summaries.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            json!({"type":"object","properties":{"histories":{"type":"array","items":{"type":"object"}}},"required":["histories"],"additionalProperties":false}),
            true,
            true,
        ),
        definition(
            "ble_evaluate",
            "Evaluate BLE time rules",
            "Evaluate disappearance of previously observed expected identities at a caller-supplied monotonic time.",
            json!({"type":"object","properties":{"now_ms":{"type":"integer","minimum":0}},"required":["now_ms"],"additionalProperties":false}),
            json!({"type":"object","properties":{"findings":{"type":"array","items":{"type":"object"}}},"required":["findings"],"additionalProperties":false}),
            false,
            false,
        ),
    ]
}

fn advertisement_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "address":{"type":"string"},
            "address_type":{"type":"string","enum":["public","random_static","resolvable_private","non_resolvable_private","unknown"]},
            "local_name":{"type":["string","null"]},
            "rssi_dbm":{"type":"integer"},
            "tx_power_dbm":{"type":["integer","null"]},
            "connectable":{"type":["boolean","null"]},
            "service_uuids":{"type":"array","items":{"type":"string"}},
            "manufacturer_data":{"type":"array","items":{"type":"object"}},
            "service_data":{"type":"array","items":{"type":"object"}},
            "protocol_identity":{"type":["string","null"]}
        },
        "required":["address","address_type","rssi_dbm"],
        "additionalProperties":false
    })
}

fn definition(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    output_schema: Value,
    read_only: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name":name,
        "title":title,
        "description":description,
        "inputSchema":input_schema,
        "outputSchema":output_schema,
        "annotations":{
            "readOnlyHint":read_only,
            "destructiveHint":false,
            "idempotentHint":idempotent,
            "openWorldHint":false
        }
    })
}

fn required<T: serde::de::DeserializeOwned>(arguments: &Value, name: &str) -> anyhow::Result<T> {
    let value = arguments
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))?;
    Ok(serde_json::from_value(value.clone())?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_state_that_identify_is_read_only() {
        let definitions = tool_definitions();
        assert_eq!(definitions[0]["name"], "ble_identify");
        assert_eq!(definitions[0]["annotations"]["readOnlyHint"], true);
        assert_eq!(definitions[2]["annotations"]["readOnlyHint"], false);
    }
}
