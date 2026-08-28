//! Text rendering for the command line.
//!
//! Only the commands a person reads at a glance get a bespoke layout. Anything
//! whose value is in its structure — a connectivity walk, a sampling run, an
//! incident bundle — is printed as indented JSON, which is more honest than a
//! table that quietly drops the nested half of the answer. `--json` forces that
//! form everywhere, so every command stays scriptable.

mod format;

use blazingly_json::Value;

use format::{compact, field, interface_errors, table};

/// Print one tool result in the form the command asked for.
pub(super) fn emit(command: &str, value: &Value, as_json: bool) -> anyhow::Result<()> {
    if as_json {
        println!("{}", blazingly_json::to_string_pretty(value)?);
        return Ok(());
    }
    let text = match command {
        "status" => status(value),
        "scan" | "networks" => networks(value),
        "analyze" => analyze(value),
        "chronicle recent" => chronicle(value),
        _ => blazingly_json::to_string_pretty(value)?,
    };
    println!("{}", text.trim_end());
    Ok(())
}

fn status(value: &Value) -> String {
    let Some(interfaces) = value.get("interfaces").and_then(Value::as_array) else {
        return "No WLAN interfaces reported.".to_string();
    };
    if interfaces.is_empty() {
        return "No WLAN interfaces reported.".to_string();
    }

    let mut out = String::new();
    for entry in interfaces {
        let interface = entry.get("interface");
        out.push_str(&format!(
            "{}\n  state: {}\n",
            field(interface, "description"),
            field(interface, "state")
        ));
        if let Some(error) = entry.get("connection_error").and_then(Value::as_str) {
            out.push_str(&format!("  error: {error}\n"));
        }
        match entry.get("connection").filter(|value| !value.is_null()) {
            None => out.push_str("  not associated\n"),
            Some(connection) => {
                out.push_str(&format!(
                    "  ssid:    {}\n  bssid:   {}\n  phy:     {}\n  \
                     signal:  {}/100  (~{} dBm)\n  rates:   rx {} kbps / tx {} kbps\n",
                    field(Some(connection), "ssid"),
                    field(Some(connection), "bssid"),
                    field(Some(connection), "phy_type"),
                    field(Some(connection), "signal_quality"),
                    field(Some(connection), "rssi_dbm_estimate"),
                    field(Some(connection), "rx_rate_kbps"),
                    field(Some(connection), "tx_rate_kbps"),
                ));
            }
        }
        out.push('\n');
    }
    out
}

fn networks(value: &Value) -> String {
    let Some(entries) = value.get("networks").and_then(Value::as_array) else {
        return "No networks in the response.".to_string();
    };

    let mut sorted: Vec<&Value> = entries.iter().collect();
    sorted.sort_by_key(|entry| {
        -entry
            .get("rssi_dbm")
            .and_then(Value::as_i64)
            .unwrap_or(-127)
    });

    let rows: Vec<Vec<String>> = sorted
        .iter()
        .map(|entry| {
            vec![
                entry
                    .get("ssid")
                    .and_then(Value::as_str)
                    .unwrap_or("<hidden>")
                    .to_string(),
                field(Some(entry), "bssid"),
                field(Some(entry), "band"),
                field(Some(entry), "channel"),
                field(Some(entry), "rssi_dbm"),
                field(Some(entry), "security"),
            ]
        })
        .collect();

    let mut out = table(&["SSID", "BSSID", "BAND", "CH", "RSSI", "SECURITY"], &rows);
    out.push_str(&format!("\n{} BSS visible", entries.len()));
    if let Some(age) = value.get("cache_age_seconds").and_then(Value::as_i64) {
        out.push_str(&format!(", scan cache {age}s old"));
    }
    out.push_str(".\n");
    out.push_str(&interface_errors(value));
    out.push_str(
        "\nSSIDs and BSSIDs identify neighbouring networks; a BSSID resolves to a street\n\
         address through public geolocation databases. Treat this list as location data.\n",
    );
    out
}

fn analyze(value: &Value) -> String {
    let Some(analysis) = value.get("analysis") else {
        return "No analysis in the response.".to_string();
    };
    let mut out = format!(
        "{} BSS analysed.\n\n",
        analysis
            .get("bss_count")
            .and_then(Value::as_i64)
            .unwrap_or_default()
    );

    if let Some(bands) = analysis.get("bands").and_then(Value::as_array) {
        let rows: Vec<Vec<String>> = bands
            .iter()
            .map(|band| {
                vec![
                    field(Some(band), "band"),
                    field(Some(band), "bss_count"),
                    field(Some(band), "distinct_ssids"),
                    field(Some(band), "distinct_channels"),
                    field(Some(band), "strongest_dbm"),
                ]
            })
            .collect();
        if !rows.is_empty() {
            out.push_str(&table(
                &["BAND", "BSS", "SSIDS", "CHANNELS", "STRONGEST"],
                &rows,
            ));
            out.push('\n');
        }
    }

    match analysis.get("findings").and_then(Value::as_array) {
        Some(findings) if !findings.is_empty() => {
            out.push_str(&format!("Findings ({}):\n\n", findings.len()));
            for finding in findings {
                out.push_str(&format!(
                    "  [{}] {}\n      {}\n\n",
                    field(Some(finding), "severity"),
                    field(Some(finding), "title"),
                    finding
                        .get("caveat")
                        .and_then(Value::as_str)
                        .unwrap_or("no caveat recorded"),
                ));
            }
        }
        _ => out.push_str("Findings: none. Nothing in the environment looks wrong.\n"),
    }
    out.push_str(&interface_errors(value));
    out
}

fn chronicle(value: &Value) -> String {
    let entries = value.get("entries").and_then(Value::as_array);
    let mut out = format!("{}\n\n", field(Some(value), "path"));
    match entries {
        Some(entries) if !entries.is_empty() => {
            for entry in entries {
                out.push_str(&format!("{}\n", compact(entry)));
            }
            out.push_str(&format!("\n{} entries", entries.len()));
        }
        _ => out.push_str("No entries recorded yet. Start one with: radiochron chronicle record"),
    }
    if let Some(invalid) = value
        .get("invalid_lines")
        .and_then(Value::as_i64)
        .filter(|count| *count > 0)
    {
        out.push_str(&format!(", {invalid} unreadable lines skipped"));
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::{analyze, networks, status};
    use blazingly_json::json;

    #[test]
    fn status_reports_an_unassociated_adapter() {
        let rendered = status(&json!({"interfaces":[{
            "interface":{"guid":"g","description":"Intel AX211","state":"disconnected"},
            "connection":null
        }]}));
        assert!(rendered.contains("Intel AX211"));
        assert!(rendered.contains("not associated"));
    }

    #[test]
    fn status_reports_an_association() {
        let rendered = status(&json!({"interfaces":[{
            "interface":{"guid":"g","description":"Intel AX211","state":"connected"},
            "connection":{"ssid":"home","bssid":"aa:bb","phy_type":"he",
                          "signal_quality":78,"rssi_dbm_estimate":-61,
                          "rx_rate_kbps":600000,"tx_rate_kbps":600000}
        }]}));
        assert!(rendered.contains("home"));
        assert!(rendered.contains("78/100"));
        assert!(rendered.contains("-61 dBm"));
    }

    #[test]
    fn networks_sort_strongest_first_and_mark_hidden() {
        let rendered = networks(&json!({
            "count":2,
            "cache_age_seconds":4,
            "networks":[
                {"ssid":null,"bssid":"a","band":"2.4","channel":6,"rssi_dbm":-80,"security":"open"},
                {"ssid":"near","bssid":"b","band":"5","channel":36,"rssi_dbm":-40,"security":"rsn"}
            ]
        }));
        let hidden = rendered.find("<hidden>").expect("hidden row");
        let near = rendered.find("near").expect("named row");
        assert!(near < hidden, "strongest BSS must sort first");
        assert!(rendered.contains("scan cache 4s old"));
        assert!(rendered.contains("location data"));
    }

    #[test]
    fn analyze_states_the_quiet_case_rather_than_printing_nothing() {
        let rendered = analyze(&json!({"analysis":{"bss_count":3,"bands":[],"findings":[]}}));
        assert!(rendered.contains("3 BSS analysed"));
        assert!(rendered.contains("none"));
    }

    #[test]
    fn analyze_lists_findings_with_their_caveat() {
        let rendered = analyze(&json!({"analysis":{"bss_count":1,"bands":[],"findings":[
            {"id":"co_channel","severity":"warning","title":"Crowded channel","caveat":"beacons only"}
        ]}}));
        assert!(rendered.contains("[warning] Crowded channel"));
        assert!(rendered.contains("beacons only"));
    }

    #[test]
    fn interface_errors_are_surfaced_not_swallowed() {
        let rendered = networks(&json!({
            "networks":[],
            "interface_errors":[{"interface_guid":"g","error_code":5}]
        }));
        assert!(rendered.contains("1 interface(s) failed"));
        assert!(rendered.contains("error_code=5"));
    }
}
