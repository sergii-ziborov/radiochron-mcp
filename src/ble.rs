use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use radiochron::ble::{Advertisement, Observation, SensorContext, Tracker, TrackerPolicy};
use serde_json::{json, Value};

use crate::mcp_server::catalog::{output_schema, tool};
use crate::mcp_server::schema::{bounded_optional_string, bounded_u64, optional_bool, required};
use crate::mcp_server::transport::RequestContext;

pub struct BleService {
    tracker: Mutex<Tracker>,
    clock_origin: Instant,
}

impl BleService {
    pub fn new() -> Self {
        Self {
            tracker: Mutex::new(Tracker::new(TrackerPolicy::default())),
            clock_origin: Instant::now(),
        }
    }

    pub fn scan(&self, arguments: &Value, context: &RequestContext) -> anyhow::Result<Value> {
        let duration_ms = bounded_u64(arguments, "duration_ms", 4_000, 500, 30_000)?;
        let sensor_id = bounded_optional_string(arguments, "sensor_id", 128)?
            .unwrap_or_else(|| "radiochron-mcp".to_string());
        let zone = bounded_optional_string(arguments, "zone", 128)?;
        let movement_session = bounded_optional_string(arguments, "movement_session", 128)?;
        let sensor_is_moving = optional_bool(arguments, "sensor_is_moving", false)?;
        let scan = crate::ble_scan::scan(duration_ms, context)?;
        let monotonic_ms = self.clock_origin.elapsed().as_millis() as u64;
        let sensor = SensorContext {
            sensor_id,
            zone,
            movement_session,
            sensor_is_moving,
        };

        let mut devices = BTreeMap::new();
        let mut findings = Vec::new();
        let advertisements_seen = scan.advertisements.len();
        for advertisement in scan.advertisements {
            let observation = Observation {
                monotonic_ms,
                unix_epoch_ms: Some(scan.observed_at_epoch_ms as i64),
                context: sensor.clone(),
                advertisement: advertisement.clone(),
            };
            let result = self.lock()?.observe(observation);
            findings.extend(result.findings.iter().cloned());
            devices.insert(
                result.identity.key.clone(),
                json!({
                    "advertisement": advertisement,
                    "identity": result.identity,
                    "payload_hash": result.payload_hash,
                    "history": result.history,
                    "findings": result.findings
                }),
            );
        }

        Ok(json!({
            "duration_ms": duration_ms,
            "observed_at_epoch_ms": scan.observed_at_epoch_ms,
            "adapters": scan.adapters,
            "advertisements_seen": advertisements_seen,
            "identities_observed": devices.len(),
            "skipped_without_rssi": scan.skipped_without_rssi,
            "devices": devices.into_values().collect::<Vec<_>>(),
            "findings": findings
        }))
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

pub fn tool_definitions(protocol_version: &str) -> Vec<Value> {
    vec![
        scan_definition(protocol_version),
        tool(
            protocol_version,
            "ble_identify",
            "Identify BLE advertisement",
            "Derive a privacy-minimized identity and payload hash from a supplied BLE advertisement.",
            json!({"type":"object","properties":{"advertisement":advertisement_schema()},"required":["advertisement"],"additionalProperties":false}),
            output_schema(&["identity","payload_hash"], json!({"identity":{"type":"object"},"payload_hash":{"type":"string"}})),
            true,
            true,
            false,
        ),
        tool(
            protocol_version,
            "ble_tracker_reset",
            "Reset BLE tracker",
            "Clear process-local BLE history and optionally apply detector thresholds, allowlist and expected identities.",
            json!({"type":"object","properties":{"policy":{"type":"object"}},"additionalProperties":false}),
            output_schema(&["reset"], json!({"reset":{"const":true}})),
            false,
            true,
            false,
        ),
        tool(
            protocol_version,
            "ble_observe",
            "Observe BLE advertisement",
            "Add one supplied timed BLE observation to process-local history and return evidence-based findings.",
            json!({"type":"object","properties":{"observation":{"type":"object"}},"required":["observation"],"additionalProperties":false}),
            output_schema(&["identity","payload_hash","history","findings"], json!({"identity":{"type":"object"},"payload_hash":{"type":"string"},"history":{"type":"object"},"findings":{"type":"array","items":{"type":"object"}}})),
            false,
            false,
            false,
        ),
        tool(
            protocol_version,
            "ble_histories",
            "BLE histories",
            "Read first/last seen, recurrence, sensor and RSSI summaries populated by native scans or explicit observations.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
            output_schema(&["histories"], json!({"histories":{"type":"array","items":{"type":"object"}}})),
            true,
            true,
            false,
        ),
        tool(
            protocol_version,
            "ble_evaluate",
            "Evaluate BLE time rules",
            "Evaluate disappearance of expected identities at a supplied monotonic time.",
            json!({"type":"object","properties":{"now_ms":{"type":"integer","minimum":0}},"required":["now_ms"],"additionalProperties":false}),
            output_schema(&["findings"], json!({"findings":{"type":"array","items":{"type":"object"}}})),
            false,
            false,
            false,
        ),
    ]
}

fn scan_definition(protocol_version: &str) -> Value {
    tool(
        protocol_version,
        "ble_scan",
        "Scan Bluetooth Low Energy",
        "Scan local Bluetooth adapters for advertisements without connecting, normalize them through RadioChron, update process-local histories, and return identity/risk evidence.",
        json!({"type":"object","properties":{
            "duration_ms":{"type":"integer","minimum":500,"maximum":30000},
            "sensor_id":{"type":"string","minLength":1,"maxLength":128},
            "zone":{"type":"string","minLength":1,"maxLength":128},
            "movement_session":{"type":"string","minLength":1,"maxLength":128},
            "sensor_is_moving":{"type":"boolean"}
        },"additionalProperties":false}),
        output_schema(
            &["duration_ms","observed_at_epoch_ms","adapters","advertisements_seen","identities_observed","skipped_without_rssi","devices","findings"],
            json!({
                "duration_ms":{"type":"integer"},"observed_at_epoch_ms":{"type":"integer"},
                "adapters":{"type":"array","items":{"type":"object"}},
                "advertisements_seen":{"type":"integer"},"identities_observed":{"type":"integer"},
                "skipped_without_rssi":{"type":"integer"},
                "devices":{"type":"array","items":{"type":"object"}},
                "findings":{"type":"array","items":{"type":"object"}}
            }),
        ),
        false,
        false,
        true,
    )
}

fn advertisement_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "address":{"type":"string"},
            "address_type":{"type":"string","enum":["public","random_static","resolvable_private","non_resolvable_private","unknown"]},
            "local_name":{"type":["string","null"]},"rssi_dbm":{"type":"integer"},
            "tx_power_dbm":{"type":["integer","null"]},"connectable":{"type":["boolean","null"]},
            "service_uuids":{"type":"array","items":{"type":"string"}},
            "manufacturer_data":{"type":"array","items":{"type":"object"}},
            "service_data":{"type":"array","items":{"type":"object"}},
            "protocol_identity":{"type":["string","null"]}
        },
        "required":["address","address_type","rssi_dbm"],
        "additionalProperties":false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_expose_native_scan_and_truthful_annotations() {
        let definitions = tool_definitions("2025-11-25");
        assert_eq!(definitions[0]["name"], "ble_scan");
        assert_eq!(definitions[0]["annotations"]["readOnlyHint"], false);
        assert_eq!(definitions[1]["annotations"]["readOnlyHint"], true);
        assert_eq!(definitions[3]["annotations"]["readOnlyHint"], false);
    }
}
