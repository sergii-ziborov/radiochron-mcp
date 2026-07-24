use std::time::Duration;

use radiochron::wlan;
use radiochron::wlan::bss::BssSummary;
use serde_json::{json, Value};

use super::super::schema::{bounded_optional_string, bounded_u64, optional_bool, optional_string};
use super::super::transport::RequestContext;
#[cfg(windows)]
use super::super::MAX_HISTORY_WINDOW_S;
use super::super::SCAN_TIMEOUT;

pub(super) fn collect_networks(arguments: &Value) -> anyhow::Result<Value> {
    let detail = optional_string(arguments, "detail")?.unwrap_or("summary");
    if !matches!(detail, "summary" | "full") {
        anyhow::bail!("detail must be summary or full");
    }
    let full = detail == "full";
    let requested_refresh = optional_bool(arguments, "refresh_scan", false)?;
    let mut refresh = if requested_refresh {
        Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?)
    } else {
        None
    };
    let mut collection = wlan::bss::bss_list_detailed()?;
    if collection.entries.is_empty() && refresh.is_none() {
        refresh = Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?);
        collection = wlan::bss::bss_list_detailed()?;
    }
    let networks = if full {
        serde_json::to_value(&collection.entries)?
    } else {
        serde_json::to_value(
            collection
                .entries
                .iter()
                .map(BssSummary::from)
                .collect::<Vec<_>>(),
        )?
    };
    Ok(json!({
        "count": collection.entries.len(),
        "detail": detail,
        "cache_age_seconds": wlan::bss::last_refresh_age_seconds(),
        "refresh": refresh,
        "interface_errors": collection.interface_errors,
        "networks": networks
    }))
}

pub(super) fn analyze_environment(arguments: &Value) -> anyhow::Result<Value> {
    let refresh = if optional_bool(arguments, "refresh_scan", false)? {
        Some(wlan::bss::scan_and_wait(SCAN_TIMEOUT)?)
    } else {
        None
    };
    let collection = wlan::bss::bss_list_detailed()?;
    let status = wlan::wifi_status()?;
    let connection = status.iter().find_map(|entry| entry.connection.as_ref());
    Ok(json!({
        "cache_age_seconds": wlan::bss::last_refresh_age_seconds(),
        "refresh": refresh,
        "interface_errors": collection.interface_errors,
        "analysis": wlan::analyze::analyze(&collection.entries, connection)
    }))
}

#[cfg(windows)]
pub(super) fn history(arguments: &Value) -> anyhow::Result<Value> {
    let within = bounded_u64(arguments, "within_seconds", 3600, 1, MAX_HISTORY_WINDOW_S)?;
    let max = bounded_u64(arguments, "max_events", 200, 1, 2000)? as usize;
    let events = radiochron::events::recent(max, Some(within))?;
    let verdict = radiochron::events::detect(&events);
    let include = optional_bool(arguments, "include_events", false)?;
    Ok(json!({
        "window_seconds": within,
        "event_count": events.len(),
        "verdict": verdict,
        "events": if include { serde_json::to_value(&events)? } else { Value::Null }
    }))
}

#[cfg(not(windows))]
pub(super) fn history(_arguments: &Value) -> anyhow::Result<Value> {
    anyhow::bail!("wifi_history is available only on Windows WLAN AutoConfig")
}

pub(super) fn sample(arguments: &Value, context: &RequestContext) -> anyhow::Result<Value> {
    let duration = bounded_u64(arguments, "duration_seconds", 20, 1, 120)?;
    let interval = bounded_u64(arguments, "interval_ms", 1000, 250, 60_000)?;
    let interface = optional_string(arguments, "interface_guid")?;
    context.check_cancelled()?;
    let run =
        wlan::sample::sample_connection_on_controlled(interface, duration, interval, |progress| {
            context.check_cancelled()?;
            context.progress(
                progress.elapsed_ms,
                progress.total_ms,
                &format!("{} samples collected", progress.sample_count),
            );
            Ok(())
        })?;
    Ok(serde_json::to_value(run)?)
}

pub(super) fn diagnose_connectivity(arguments: &Value) -> anyhow::Result<Value> {
    let config = connectivity_config(arguments)?;
    Ok(serde_json::to_value(radiochron::connectivity::diagnose(
        &config,
    ))?)
}

pub(super) fn validate_connectivity(arguments: &Value) -> anyhow::Result<()> {
    connectivity_config(arguments).map(|_| ())
}

fn connectivity_config(
    arguments: &Value,
) -> anyhow::Result<radiochron::connectivity::ConnectivityConfig> {
    Ok(radiochron::connectivity::ConnectivityConfig {
        dns_name: bounded_optional_string(arguments, "dns_name", 253)?,
        tcp_target: bounded_optional_string(arguments, "tcp_target", 512)?,
        internet_target: bounded_optional_string(arguments, "internet_target", 512)?,
        captive_portal_url: bounded_optional_string(arguments, "captive_portal_url", 2048)?,
        captive_portal_expected_status: bounded_u64(
            arguments,
            "captive_portal_expected_status",
            204,
            100,
            599,
        )? as u16,
        tls_target: bounded_optional_string(arguments, "tls_target", 512)?,
        quality_target: bounded_optional_string(arguments, "quality_target", 512)?,
        quality_attempts: bounded_u64(arguments, "quality_attempts", 4, 1, 20)? as u8,
        timeout: Duration::from_millis(bounded_u64(arguments, "timeout_ms", 3_000, 100, 30_000)?),
    })
}
