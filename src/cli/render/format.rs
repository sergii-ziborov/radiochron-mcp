//! Shared text primitives: fixed-width tables and safe field access.
//!
//! Every accessor here tolerates a missing or differently-typed field and
//! prints `-` instead. A renderer must not panic on a payload shape it did not
//! expect — a diagnostic tool that crashes while describing a broken adapter is
//! worse than one that prints a dash.

use blazingly_json::Value;

/// A scan that partly failed must say so; an empty list otherwise reads as
/// quiet air rather than a driver that refused.
pub(super) fn interface_errors(value: &Value) -> String {
    let Some(errors) = value.get("interface_errors").and_then(Value::as_array) else {
        return String::new();
    };
    if errors.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n{} interface(s) failed during collection:\n",
        errors.len()
    );
    for error in errors {
        out.push_str(&format!("  {}\n", compact(error)));
    }
    out
}

/// One line of `key=value` for an object whose shape the CLI does not model.
pub(super) fn compact(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return scalar(value);
    };
    object
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(name, value)| format!("{name}={}", scalar(value)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// One named field of an object that may itself be absent.
pub(super) fn field(value: Option<&Value>, name: &str) -> String {
    value
        .and_then(|value| value.get(name))
        .map_or_else(|| "-".to_string(), scalar)
}

pub(super) fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "-".to_string(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Columns padded to their widest cell, measured in characters rather than
/// bytes so a non-ASCII SSID does not skew the table.
pub(super) fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }

    let line = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let width = widths.get(index).copied().unwrap_or_default();
                let padding = width.saturating_sub(cell.chars().count());
                format!("{cell}{}", " ".repeat(padding))
            })
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };

    let mut out = line(&headers.iter().map(|h| (*h).to_string()).collect::<Vec<_>>());
    out.push('\n');
    for row in rows {
        out.push_str(&line(row));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{compact, field, scalar, table};
    use blazingly_json::json;

    #[test]
    fn table_pads_columns_to_the_widest_cell() {
        let rendered = table(
            &["A", "B"],
            &[
                vec!["long-value".into(), "x".into()],
                vec!["s".into(), "y".into()],
            ],
        );
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "A           B");
        assert_eq!(lines[1], "long-value  x");
        assert_eq!(lines[2], "s           y");
    }

    #[test]
    fn table_widths_count_characters_not_bytes() {
        let rendered = table(&["SSID", "CH"], &[vec!["кафе".into(), "6".into()]]);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "SSID  CH");
        assert_eq!(lines[1], "кафе  6");
    }

    #[test]
    fn missing_and_mistyped_fields_render_as_a_dash() {
        let value = json!({"present": "here", "empty": null});
        assert_eq!(field(Some(&value), "present"), "here");
        assert_eq!(field(Some(&value), "empty"), "-");
        assert_eq!(field(Some(&value), "absent"), "-");
        assert_eq!(field(None, "anything"), "-");
        assert_eq!(scalar(&json!(42)), "42");
        assert_eq!(scalar(&json!(true)), "true");
    }

    #[test]
    fn compact_skips_nulls_and_falls_back_for_scalars() {
        assert_eq!(
            compact(&json!({"code": 5, "detail": null, "name": "wlan0"})),
            "code=5 name=wlan0"
        );
        assert_eq!(compact(&json!("plain")), "plain");
    }
}
