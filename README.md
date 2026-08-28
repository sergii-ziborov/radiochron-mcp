# radiochron-mcp

**[radiochron.com](https://radiochron.com)** · the chronicle of your radio.

A local-first [Model Context Protocol](https://modelcontextprotocol.io) server
**and command line** for Wi-Fi incident diagnosis and Bluetooth Low Energy
observation. It combines native radio collection with the
[`radiochron`](https://github.com/sergii-ziborov/radiochron) Rust core, then
returns typed conclusions instead of forcing an assistant to interpret raw
operating-system output.

The two front ends are not two implementations. `radiochron analyze` and the
`wifi_analyze` tool run the same handler with the same bounds and the same error
text, so a conclusion never depends on how you asked for it.

The preferred MCP revision is `2025-11-25`; clients that request
`2025-06-18` receive the compatible legacy tool shape. Every tool has input and
output schemas, structured content, safety annotations, explicit execution
semantics, cancellation where work is long-running, and actionable separation
between JSON-RPC protocol errors and tool execution errors.

RadioChron repositories remain independent:

- [`radiochron`](https://github.com/sergii-ziborov/radiochron) is the Rust/IoT core.
- [`radiochron-js`](https://github.com/sergii-ziborov/radiochron-js) is the Node/npm library; it does not ship MCP.
- [`radiochron-mcp`](https://github.com/sergii-ziborov/radiochron-mcp) is this pure-Rust MCP server and command line.
- [`radiochron-agent`](https://github.com/sergii-ziborov/radiochron-agent) is the unattended durable collector/exporter and does not depend on MCP.
- [`radiochron-electron`](https://github.com/sergii-ziborov/radiochron-electron) is the standalone desktop app and does not depend on MCP.

## Install

The npm package carries verified native binaries for Windows x64, Linux
x64/ARM64, Intel Mac, and Apple Silicon:

```sh
claude mcp add radiochron -- npx -y radiochron-mcp
```

Or install the Rust binary from source:

```sh
cargo install --git https://github.com/sergii-ziborov/radiochron-mcp
```

Building on Debian/Ubuntu requires `libdbus-1-dev` and `pkg-config` for the
BlueZ adapter. Prebuilt npm users do not need a Rust toolchain.

Register an installed binary with any stdio MCP client:

```json
{
  "mcpServers": {
    "radiochron": {
      "command": "radiochron"
    }
  }
}
```

`RADIOCHRON_CHRONICLE_PATH` optionally overrides the local chronicle path:
`%LOCALAPPDATA%\RadioChron` on Windows, `~/Library/Application
Support/RadioChron` on macOS, or the XDG state directory on Linux.

## Command line

The same binary is a CLI. Nothing extra to install — `npx radiochron status`
works from the npm package, and `cargo install` puts `radiochron` on `PATH`.

```sh
radiochron status                  # association state of every adapter
radiochron scan                    # look again, then list what is there
radiochron analyze                 # findings about the environment
radiochron report                  # the full Markdown diagnostic report
radiochron incident --ble          # one composite answer for "the Wi-Fi is broken"
radiochron connectivity --dns example.com --tcp example.com:443
radiochron ble scan --duration 4000
radiochron chronicle recent --max 20
radiochron --help                  # every command and flag
```

Add `--json` to any command for the raw tool result, which is what a script
wants:

```sh
radiochron networks --json | jq '.networks[] | select(.rssi_dbm > -60) | .ssid'
```

`radiochron tool <name> [--args '<json>']` reaches any tool the catalogue
publishes, including ones with no friendly spelling:

```sh
radiochron tool ble_evaluate --args '{"now_ms":1717000000000}'
```

Unknown flags are refused rather than ignored, so a mistyped `--durations`
fails loudly instead of silently running with a default.

**Running with no arguments still serves MCP over stdio**, exactly as before, so
every existing client configuration keeps working. A terminal on stdin means a
person typed the name, and prints help instead; a pipe means a client, and
speaks the protocol. `radiochron mcp` forces the server explicitly.

Two commands are CLI-only, because they have no session to live in:
`chronicle record` runs the recorder in the foreground until you stop it, and
`chronicle path` prints where the journal is kept.

## Start with one tool

Use `diagnose_incident` first. One request returns independent sections for:

- current Wi-Fi interfaces and association;
- RF/environment analysis;
- radio → authentication → DHCP → gateway → DNS → TCP → Internet stages;
- Windows WLAN event history when available;
- recent change-only chronicle entries;
- an optional native BLE advertisement scan, normalized identities, retained
  histories, and evidence-based findings.

One unavailable collector does not discard the rest of the incident. Each
section has `ok`, `data`, or an actionable `error`, and the top-level
`problems` list is compact enough for an assistant to explain directly.
Targets are never contacted unless the caller supplies them.

## Tool surface

Seventeen tools are portable. Windows exposes an eighteenth,
`wifi_history`, backed by WLAN AutoConfig.

| Tool | Purpose |
|---|---|
| `diagnose_incident` | Orchestrate Wi-Fi, connectivity, history, chronicle, and optional native BLE evidence in one compact response |
| `wifi_status` | Current state of every WLAN interface |
| `wifi_networks` | Nearby BSS records with real dBm, security, width, and load; summary or full detail |
| `wifi_analyze` | Signal, contention, roaming, security, and scan-quality findings |
| `wifi_history` (Windows) | Reconnect loops, key-exchange failures, and credential-mismatch evidence |
| `wifi_sample` | Cancelable RSSI/rate/roaming sampling with progress |
| `wifi_scan` | Native Wi-Fi refresh with per-interface completion/failure |
| `connectivity_diagnose` | Separate radio, authentication, IP assignment, gateway, DNS, TCP, portal, TLS, quality, and Internet stages |
| `chronicle_start` | Start the local rotating change-only JSONL recorder |
| `chronicle_stop` | Stop and flush the recorder |
| `chronicle_status` | Recorder state, path, counters, and latest error |
| `chronicle_recent` | Recent entries from active and rotated files |
| `ble_scan` | Scan native adapters without connecting, normalize advertisements, update histories, and return risk evidence |
| `ble_identify` | Identify a caller-supplied advertisement and hash its payload |
| `ble_tracker_reset` | Clear process-local BLE history and apply detector policy |
| `ble_observe` | Add an externally collected timed observation |
| `ble_histories` | First/last seen, recurrence, sensors, movement sessions, and RSSI summaries |
| `ble_evaluate` | Time-based disappearance findings for expected identities |

`ble_scan` uses WinRT on Windows, BlueZ on Linux, and CoreBluetooth on macOS.
It listens only for devices observed during the requested scan window, does
not perform GATT connections, and feeds the same privacy-minimized RadioChron
tracker used by explicit `ble_observe` calls. iBeacon and Eddystone UID data
can provide stronger protocol identity; generic private addresses remain
ephemeral.

On macOS 11+, the host application or terminal launching the MCP process must
have Bluetooth permission. An app bundle needs
`NSBluetoothAlwaysUsageDescription`; a terminal-launched server requires
Bluetooth access for that terminal in System Settings. Linux requires a
running BlueZ service and access to the system D-Bus.

## MCP behavior

- Newline-delimited UTF-8 JSON-RPC 2.0 over stdio; stdout contains MCP frames only.
- Negotiates both `2025-11-25` and `2025-06-18`, preferring the current revision.
- Current tool definitions declare `execution.taskSupport: "forbidden"` because
  this local stdio server uses normal cancelable requests rather than durable
  experimental tasks.
- Unknown methods and malformed call envelopes use JSON-RPC errors.
- Tool input/radio/platform failures use `isError: true` so a model can correct
  arguments or explain the platform problem.
- Structured results are also serialized into text content for older clients.
- Source files are architecture-gated at 300 lines; real stdio conformance
  tests cover current and legacy lifecycle/catalog/error behavior.

The tools and resource request paths use the Tokio-free `mcport` runtime and
its `blazingly-json` value/codec surface. RadioChron keeps only its strict
initialize/notification lifecycle locally. Native BLE collection uses the
separate `radiochron-native-ble` crate, which talks directly to WinRT, BlueZ
D-Bus, and CoreBluetooth without `btleplug`, Tokio, or `futures`. The portable
`radiochron` core remains independent of host Bluetooth APIs.

## Safety and privacy

SSIDs, BSSIDs, Bluetooth addresses, advertisement payloads, and event logs can
be sensitive. The server has no telemetry and sends nothing off the machine.
The chronicle writes only its rotating local JSONL file. Saved Wi-Fi passwords
are never read.

RSSI is signal evidence, not physical distance or direction. Private Bluetooth
addresses can rotate, so clone/recurrence claims require protocol identity or
caller-provided identity. Native BLE scanning never connects to peripherals.

The MCP surface intentionally excludes plaintext Wi-Fi keys, adapter MAC
changes, adapter restarts, computer rename, active LAN sweeps, arbitrary shell
execution, and external AI review.

## Release

Releases are assembled from one green cross-platform CI run. The npm archive
must contain revision-matched binaries and provenance sidecars for all five
targets. An immutable version tag publishes the npm package through the
protected repository secret, then publishes the matching `server.json` to the
official MCP Registry through GitHub OIDC. The npm credential is the masked,
tag-gated `NPM_TOKEN` GitHub Actions secret; the MCP Registry uses no
long-lived token.

## License

Licensed under the [MIT License](LICENSE-MIT). The underlying `radiochron` Rust
core remains separately dual-licensed under MIT or Apache-2.0.
