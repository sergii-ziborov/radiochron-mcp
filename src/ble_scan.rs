use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use btleplug::api::{
    AddressType as NativeAddressType, Central, CentralEvent, Manager as _, Peripheral, ScanFilter,
};
use btleplug::platform::Manager;
use futures::StreamExt;
use radiochron::ble::{AddressType, Advertisement, ManufacturerData, ServiceData};
use serde::Serialize;

use crate::mcp_server::transport::RequestContext;

#[derive(Debug, Serialize)]
pub struct AdapterReport {
    pub name: String,
    pub state: String,
    pub scan_started: bool,
    pub errors: Vec<String>,
}

pub struct NativeScan {
    pub observed_at_epoch_ms: u64,
    pub adapters: Vec<AdapterReport>,
    pub advertisements: Vec<Advertisement>,
    pub skipped_without_rssi: usize,
}

pub fn scan(duration_ms: u64, context: &RequestContext) -> anyhow::Result<NativeScan> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    runtime
        .block_on(scan_async(duration_ms, context))
        .map_err(annotate_platform_error)
}

async fn scan_async(duration_ms: u64, context: &RequestContext) -> anyhow::Result<NativeScan> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        anyhow::bail!("no Bluetooth adapters found");
    }

    let mut reports = Vec::with_capacity(adapters.len());
    let mut started = Vec::new();
    let mut events = futures::stream::SelectAll::new();
    for (index, adapter) in adapters.iter().enumerate() {
        let name = adapter
            .adapter_info()
            .await
            .unwrap_or_else(|_| "Bluetooth adapter".to_string());
        let state = adapter
            .adapter_state()
            .await
            .map(|state| format!("{state:?}"))
            .unwrap_or_else(|error| format!("Unknown ({error})"));
        let mut report = AdapterReport {
            name,
            state,
            scan_started: false,
            errors: Vec::new(),
        };
        let event_stream = adapter.events().await;
        match (
            event_stream,
            adapter.start_scan(ScanFilter::default()).await,
        ) {
            (Ok(stream), Ok(())) => {
                report.scan_started = true;
                started.push(true);
                events.push(stream.map(move |event| (index, event)).boxed());
            }
            (Err(error), _) => {
                report.errors.push(format!("open event stream: {error}"));
                started.push(false);
            }
            (_, Err(error)) => {
                report.errors.push(format!("start scan: {error}"));
                started.push(false);
            }
        }
        reports.push(report);
    }
    if !started.iter().any(|started| *started) {
        anyhow::bail!("Bluetooth scan could not start on any adapter");
    }

    let started_at = Instant::now();
    let duration = Duration::from_millis(duration_ms);
    let mut observed = (0..adapters.len())
        .map(|_| std::collections::BTreeSet::new())
        .collect::<Vec<_>>();
    let mut cancellation = None;
    while started_at.elapsed() < duration {
        if let Err(error) = context.check_cancelled() {
            cancellation = Some(error);
            break;
        }
        let elapsed = started_at.elapsed().min(duration);
        context.progress(
            elapsed.as_millis(),
            duration.as_millis(),
            "collecting BLE advertisements",
        );
        let remaining = duration.saturating_sub(started_at.elapsed());
        tokio::select! {
            event = events.next() => {
                if let Some((adapter_index, event)) = event {
                    let id = match event {
                        CentralEvent::DeviceDiscovered(id)
                        | CentralEvent::DeviceUpdated(id)
                        | CentralEvent::ManufacturerDataAdvertisement { id, .. }
                        | CentralEvent::ServiceDataAdvertisement { id, .. }
                        | CentralEvent::ServicesAdvertisement { id, .. } => Some(id),
                        _ => None,
                    };
                    if let Some(id) = id {
                        observed[adapter_index].insert(id);
                    }
                }
            }
            _ = tokio::time::sleep(remaining.min(Duration::from_millis(250))) => {}
        }
    }

    let mut advertisements = Vec::new();
    let mut skipped_without_rssi = 0;
    for (index, adapter) in adapters.iter().enumerate() {
        if !started[index] {
            continue;
        }
        if let Err(error) = adapter.stop_scan().await {
            reports[index].errors.push(format!("stop scan: {error}"));
        }
        for id in &observed[index] {
            let peripheral = match adapter.peripheral(id).await {
                Ok(peripheral) => peripheral,
                Err(error) => {
                    reports[index]
                        .errors
                        .push(format!("resolve observed peripheral: {error}"));
                    continue;
                }
            };
            match peripheral.properties().await {
                Ok(Some(properties)) => {
                    let Some(rssi_dbm) = properties.rssi else {
                        skipped_without_rssi += 1;
                        continue;
                    };
                    advertisements.push(to_advertisement(
                        peripheral.id().to_string(),
                        properties,
                        rssi_dbm,
                    ));
                }
                Ok(None) => {}
                Err(error) => reports[index]
                    .errors
                    .push(format!("read peripheral properties: {error}")),
            }
        }
    }
    if let Some(error) = cancellation {
        return Err(error);
    }

    Ok(NativeScan {
        observed_at_epoch_ms: epoch_millis(),
        adapters: reports,
        advertisements,
        skipped_without_rssi,
    })
}

fn to_advertisement(
    peripheral_id: String,
    properties: btleplug::api::PeripheralProperties,
    rssi_dbm: i16,
) -> Advertisement {
    let address_type = match properties.address_type {
        Some(NativeAddressType::Public) => AddressType::Public,
        Some(NativeAddressType::Random) | None => AddressType::Unknown,
    };
    #[cfg(target_vendor = "apple")]
    let address = peripheral_id;
    #[cfg(not(target_vendor = "apple"))]
    let address = properties.address.to_string();
    #[cfg(not(target_vendor = "apple"))]
    let _ = peripheral_id;

    let mut manufacturer_data = properties
        .manufacturer_data
        .into_iter()
        .map(|(company_id, data)| ManufacturerData { company_id, data })
        .collect::<Vec<_>>();
    manufacturer_data.sort_by_key(|item| item.company_id);
    let mut service_data = properties
        .service_data
        .into_iter()
        .map(|(uuid, data)| ServiceData {
            uuid: uuid.to_string(),
            data,
        })
        .collect::<Vec<_>>();
    service_data.sort_by(|left, right| left.uuid.cmp(&right.uuid));
    let mut service_uuids = properties
        .services
        .into_iter()
        .map(|uuid| uuid.to_string())
        .collect::<Vec<_>>();
    service_uuids.sort();

    Advertisement {
        address,
        address_type,
        local_name: properties.local_name,
        rssi_dbm,
        tx_power_dbm: properties.tx_power_level,
        connectable: None,
        service_uuids,
        manufacturer_data,
        service_data,
        protocol_identity: None,
    }
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn annotate_platform_error(error: anyhow::Error) -> anyhow::Error {
    if error.to_string().contains("request cancelled by client") {
        return error;
    }
    #[cfg(target_vendor = "apple")]
    return anyhow::anyhow!(
        "native BLE scan failed: {error}. Grant Bluetooth permission to the MCP host/terminal; app bundles also need NSBluetoothAlwaysUsageDescription"
    );
    #[cfg(target_os = "linux")]
    return anyhow::anyhow!(
        "native BLE scan failed: {error}. Ensure BlueZ is running and the process can access the system D-Bus"
    );
    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    anyhow::anyhow!("native BLE scan failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn native_properties_are_normalized_and_sorted() {
        let mut properties = btleplug::api::PeripheralProperties {
            address: "01:02:03:04:05:06".parse().unwrap(),
            address_type: Some(NativeAddressType::Public),
            local_name: Some("sensor".into()),
            rssi: Some(-42),
            ..Default::default()
        };
        properties.manufacturer_data = HashMap::from([(2, vec![2]), (1, vec![1])]);
        let advertisement = to_advertisement("native-id".into(), properties, -42);
        assert_eq!(advertisement.rssi_dbm, -42);
        assert_eq!(advertisement.manufacturer_data[0].company_id, 1);
        assert_eq!(advertisement.address_type, AddressType::Public);
    }
}
