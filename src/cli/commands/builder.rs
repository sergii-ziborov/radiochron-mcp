//! Assembles a tool's argument object from the flags that were actually given.

use blazingly_json::{Map, Value};

/// Collects only the arguments the caller supplied.
///
/// An absent flag must stay absent rather than arrive as `null`: the tools
/// distinguish "not asked for" from "asked for, empty", and their documented
/// defaults apply to the first case only. Sending `null` would turn every
/// unset flag into a type error at the handler.
#[derive(Default)]
pub(super) struct Builder {
    fields: Vec<(String, Value)>,
}

impl Builder {
    pub(super) fn set(&mut self, name: &str, value: Value) {
        self.fields.push((name.to_string(), value));
    }

    pub(super) fn text(&mut self, name: &str, value: Option<&str>) {
        if let Some(value) = value {
            self.set(name, Value::String(value.to_string()));
        }
    }

    pub(super) fn integer(&mut self, name: &str, value: Option<i64>) {
        if let Some(value) = value {
            self.set(name, Value::from(value));
        }
    }

    /// A switch reaches the tool only when it is on: the handlers already
    /// default the absent case to `false`.
    pub(super) fn flag(&mut self, name: &str, on: bool) {
        if on {
            self.set(name, Value::Bool(true));
        }
    }

    pub(super) fn build(self) -> Value {
        Value::Object(Map::from_iter(self.fields))
    }
}

#[cfg(test)]
mod tests {
    use super::Builder;

    #[test]
    fn empty_builds_an_object_not_null() {
        let value = Builder::default().build();
        assert!(value.as_object().expect("object").is_empty());
    }

    #[test]
    fn only_supplied_values_appear() {
        let mut builder = Builder::default();
        builder.text("zone", Some("lab"));
        builder.text("sensor_id", None);
        builder.integer("duration_ms", Some(2500));
        builder.integer("ble_scan_ms", None);
        builder.flag("include_ble", true);
        builder.flag("sensor_is_moving", false);

        let value = builder.build();
        assert_eq!(value["zone"], "lab");
        assert_eq!(value["duration_ms"], 2500);
        assert_eq!(value["include_ble"], true);
        assert_eq!(value.as_object().expect("object").len(), 3);
    }
}
