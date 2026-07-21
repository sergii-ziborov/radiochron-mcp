# radiochron-mcp

**[radiochron.com](https://radiochron.com)** · the chronicle of your radio.

A [Model Context Protocol](https://modelcontextprotocol.io) server that gives an
AI assistant local Wi-Fi diagnostics over stdio: connection-history **verdicts**
(reconnect loops, an AP failing key exchange, a credential mismatch), findings
instead of data dumps, live signal sampling, and native WLAN collectors.

Built on the [`radiochron`](https://crates.io/crates/radiochron) library. Pure
Rust, **no MCP SDK**, three dependencies, a ~1.0 MB binary, and **no build
toolchain beyond a stock [`rustup`](https://rustup.rs)**. The optional chronicle
writes only its rotating local JSONL file; saved passwords are never read and
nothing leaves the machine.

## Install

The crate is `radiochron-mcp`; the binary it installs is named `radiochron`,
because a `radiochron-mcp.exe` in a client config reads as internal plumbing.

```sh
cargo install radiochron-mcp
```

No Rust toolchain? The [`radiochron`](https://www.npmjs.com/package/radiochron)
npm package ships the same binary prebuilt:

```sh
claude mcp add radiochron -- npx -y radiochron
```

## Register with an MCP client

For Claude Code, point it at the installed binary:

```sh
claude mcp add radiochron -- radiochron
```

Or add it to any MCP client config directly:

```json
{
  "mcpServers": {
    "radiochron": { "command": "radiochron" }
  }
}
```

No arguments are required. `RADIOCHRON_CHRONICLE_PATH` optionally overrides
the default `%LOCALAPPDATA%\RadioChron\chronicle.jsonl` path. The transport is
newline-delimited JSON-RPC 2.0 over stdio.

## Tools

Ten tools with machine-readable input/output schemas, structured results and
truthful MCP safety annotations.

| Tool | Arguments | Returns |
|---|---|---|
| `wifi_status` | — | Every WLAN interface and, for the associated one: SSID, BSSID, PHY type (`ht`/`vht`/`he`/`eht`), signal quality, estimated RSSI in dBm, rx/tx rates |
| `wifi_networks` | `refresh_scan?: boolean`<br>`detail?: "summary" \| "full"` | Nearby BSS plus cache age, scan completion, per-interface errors, WPA2/WPA3/OWE, cipher, PMF, width and load fields |
| `wifi_analyze` | `refresh_scan?: boolean` | **Findings, not records.** Co-channel contention, crowded-channel association, weak signal, band-steering and roam candidates, insecure security, hidden SSIDs, scan-quality problems |
| `wifi_history` | `within_seconds?: number`<br>`max_events?: number`<br>`include_events?: boolean` | **Why it dropped earlier.** Reads the WLAN AutoConfig event log and returns a verdict: reconnect loops, an AP repeatedly failing key exchange, a suspected credential mismatch |
| `wifi_sample` | `interface_guid?: string`<br>`duration_seconds?: 1..120`<br>`interval_ms?: 250..60000` | Cancelable sampling with progress; collector errors remain distinct from disconnects |
| `wifi_scan` | — | Triggers a standard scan and waits for each Windows completion/failure notification |
| `chronicle_start` | `interval_seconds?: 1..300`<br>`signal_threshold_db?: 1..50` | Starts the local rotating change-only recorder |
| `chronicle_stop` | — | Stops and flushes the recorder |
| `chronicle_status` | — | Recorder state, path and latest error |
| `chronicle_recent` | `max_entries?: 1..1000` | Recent entries from active and rotated files |

**Prefer `wifi_analyze`.** On a real 43-BSS environment it answers in ~800 bytes
where the full BSS list costs ~41 KB — because it returns the conclusion, not
the evidence. Every finding carries a `caveat` field stating why it might be
wrong; that is part of the payload on purpose, since a bare severity invites
over-trust and several of these signals are genuinely weaker than they look.

Two behaviours worth knowing about `wifi_networks`: the driver cache can be
empty, so a first empty read is retried once behind a real scan (the `refresh`
object says what completed) rather than reported as "no networks"; and `summary`
is the default — ask for `full` (raw IEs, rates, capability bits) only when you
need those fields.

## Deliberately not exposed

The parent project grew collectors that are unsafe to hand to an autonomous
model. They are **not** part of this server's tool surface, and calling them
returns `-32601 unknown tool`:

- **plaintext saved Wi-Fi keys** — a model must not be able to read and leak credentials
- **adapter MAC change / adapter restart / computer rename** — privileged, disruptive, can drop the operator off the network
- **active LAN sweeps** — emit probe traffic, trip IDS on managed segments
- **external AI-review shell-out** — arbitrary process execution and off-box data flow

## Why no SDK

The official `rmcp` crate would add `tokio`, `schemars`, and a mandatory
`chrono` whose `clock` feature depends on `windows-link`/`raw-dylib` —
reintroducing the exact build requirement this project exists to avoid. The
stdio transport is a few hundred lines over `serde_json` instead, so the server
builds on nothing but `rustup`.

## Platform

Windows-only today — the engine talks to `wlanapi.dll` and the WLAN event log
directly. Linux (nl80211) and macOS (CoreWLAN) are on the
[roadmap](https://radiochron.com/#roadmap).

## Safety and privacy

SSIDs, BSSIDs, MAC addresses and event logs are sensitive. This server is
local-first, has no telemetry, and transmits nothing off the machine. Only run
scans against networks you own or are authorized to test. It is not a packet
sniffer, a geolocation system, or offensive Wi-Fi tooling.

## License

Licensed under either of [Apache-2.0](https://github.com/sergii-ziborov/radiochron/blob/main/LICENSE-APACHE)
or [MIT](https://github.com/sergii-ziborov/radiochron/blob/main/LICENSE-MIT), at
your option.
