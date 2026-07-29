use std::time::Duration;

use radiochron_native_ble::{ScanObserver, ScanReport};

use crate::mcp_server::transport::RequestContext;

pub type NativeScan = ScanReport;

pub fn scan(duration_ms: u64, context: &RequestContext) -> anyhow::Result<NativeScan> {
    let observer = ContextObserver(context);
    let report =
        radiochron_native_ble::scan_with_observer(Duration::from_millis(duration_ms), &observer)
            .map_err(annotate_platform_error)?;
    context.check_cancelled()?;
    Ok(report)
}

struct ContextObserver<'a>(&'a RequestContext);

impl ScanObserver for ContextObserver<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.check_cancelled().is_err()
    }

    fn progress(&self, elapsed: Duration, total: Duration) {
        self.0.progress(
            elapsed.as_millis(),
            total.as_millis(),
            "collecting BLE advertisements",
        );
    }
}

fn annotate_platform_error(error: radiochron_native_ble::Error) -> anyhow::Error {
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
