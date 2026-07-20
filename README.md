# radiochron-mcp

**[radiochron.com](https://radiochron.com)** · the chronicle of your radio.

A [Model Context Protocol](https://modelcontextprotocol.io) server that gives an
AI assistant local Wi-Fi diagnostics over stdio: connection-history **verdicts**
(reconnect loops, an AP failing key exchange, a credential mismatch), findings
instead of data dumps, live signal sampling, and native WLAN collectors.

Built on the [`radiochron`](https://crates.io/crates/radiochron) library. Pure
Rust, **no MCP SDK**, three dependencies, a ~724 KB binary, and **no build
toolchain beyond a stock [`rustup`](https://rustup.rs)**. Nothing writes,
nothing reads saved passwords, nothing leaves the machine.

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

No arguments, no configuration, no environment variables. The transport is
newline-delimited JSON-RPC 2.0 over stdio.

## Tools

Six read-only tools. Nothing here changes system state.

| Tool | Arguments | Returns |
|---|---|---|
| `wifi_status` | — | Every WLAN interface and, for the associated one: SSID, BSSID, PHY type (`ht`/`vht`/`he`/`eht`), signal quality, estimated RSSI in dBm, rx/tx rates |
| `wifi_networks` | `refresh_scan?: boolean`<br>`detail?: "summary" \| "full"` | `{count, refreshed, detail, networks}` — nearby BSS entries with SSID, BSSID, band, channel, real RSSI in dBm, PHY type, security and capability flags |
| `wifi_analyze` | `refresh_scan?: boolean` | **Findings, not records.** Co-channel contention, crowded-channel association, weak signal, band-steering and roam candidates, insecure security, hidden SSIDs, scan-quality problems |
| `wifi_history` | `within_seconds?: number`<br>`max_events?: number`<br>`include_events?: boolean` | **Why it dropped earlier.** Reads the WLAN AutoConfig event log and returns a verdict: reconnect loops, an AP repeatedly failing key exchange, a suspected credential mismatch |
| `wifi_sample` | `duration_seconds?: 1..120`<br>`interval_ms?: >=250` | Connection dynamics over a window: RSSI min/max/mean and swing, rx-rate range, distinct BSSIDs, roam count, disconnected samples |
| `wifi_scan` | — | Triggers a driver scan on each interface; returns how many accepted |

**Prefer `wifi_analyze`.** On a real 43-BSS environment it answers in ~800 bytes
where the full BSS list costs ~41 KB — because it returns the conclusion, not
the evidence. Every finding carries a `caveat` field stating why it might be
wrong; that is part of the payload on purpose, since a bare severity invites
over-trust and several of these signals are genuinely weaker than they look.

Two behaviours worth knowing about `wifi_networks`: the driver cache can be
empty, so a first empty read is retried once behind a real scan (the `refreshed`
field says whether it was) rather than reported as "no networks"; and `summary`
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
