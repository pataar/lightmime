# AGENTS.md

Guidance for coding agents working in this repository. User-facing docs are in
[README.md](README.md).

## What this is

lightmime syncs Philips Hue lights with the screen on Linux. It captures the
screen via the XDG ScreenCast portal + PipeWire, maps it to one colour per Hue
Entertainment channel, and streams those to the bridge over DTLS. A daemon
exposes it to Home Assistant through MQTT discovery. A GUI is planned and must
be able to reuse the core without touching the daemon.

## Layout

```
crates/lightmime-core/     library, no UI, no MQTT
  src/config.rs            Config, BridgeConfig, CaptureConfig, SyncSettings (serde, defaults, sanitized())
  src/capture.rs           portal session + GStreamer pipeline (pipewiresrc → appsink), FrameSource
  src/color.rs             Frame, Zone (channel position → screen point), ColorMapper (sample, saturate, smooth, brightness)
  src/hue/api.rs           CLIP v2 REST: pair(), Bridge (areas, start/stop streaming, PSK identity)
  src/hue/stream.rs        EntertainmentStream: DTLS 1.2 PSK + HueStream v2 messages (blocking)
  src/engine.rs            Engine handle + actor; owns capture and bridge session, runs the stream thread
crates/lightmime/          the `lightmime` binary
  src/main.rs              CLI: pair, import-lumux, areas, use-area, test, run
  src/settings.rs          AppConfig = core Config (flattened) + [mqtt]; load/save (0600, atomic)
  src/mqtt.rs              Home Assistant MQTT discovery bridge
contrib/                   example config, systemd user unit
```

## Architecture rules

- `lightmime-core` stays frontend-agnostic: no MQTT, CLI or GUI code there.
  Frontends interact only through `Engine` (`spawn`, `start`, `stop`,
  `status`/`subscribe`, `settings`/`update_settings`). A future
  `crates/lightmime-gui` embeds `Engine` the same way the daemon does.
- The engine is an actor on the tokio runtime; frame streaming runs on a
  dedicated std thread (`StreamWorker`) because DTLS I/O is blocking and the
  loop is time-critical. Live settings are shared through `Arc<RwLock<SyncSettings>>`
  and read every frame; status goes out through a `tokio::sync::watch` channel.
- Stream errors are reported back to the actor (`Command::StreamFailed`), which
  stops the entertainment area and capture and moves to `EngineState::Error`.
  Always stop the area on the bridge when a session ends, or the lights stay
  locked in streaming mode.
- `SyncSettings::sanitized()` is the single place values get clamped; call it
  whenever settings change.

## Protocol notes

- Pairing: `POST /api` with `generateclientkey: true` returns `username`
  (app key) and `clientkey` (32 hex chars = the PSK). Error type 101 means the
  link button was not pressed.
- Streaming: `PUT /clip/v2/resource/entertainment_configuration/{id}` with
  `{"action":"start"}` first, then DTLS to UDP 2100, cipher
  `PSK-AES128-GCM-SHA256`. PSK identity is the `hue-application-id` header from
  `GET /auth/v1`, falling back to the app key.
- Message: `HueStream`, version `02 00`, sequence, `00 00`, colour space
  (`00` = RGB), `00`, 36-byte area UUID, then per channel `id` + 3 × u16 BE.
  Max 20 channels. One `ssl_write` = one datagram (see `Datagram` in
  `stream.rs`); never buffer several messages into one write.
- The bridge drops the session after ~10 s without data, so the loop sends
  black until the first frame arrives.
- Channel positions: x = left/right, y = front/back (ignored), z = bottom/top,
  all in -1..1. `Zone::from_channel` maps x and z to screen coordinates.
- The bridge uses a certificate from Signify's own CA; the REST client accepts
  invalid certs on purpose (LAN only).

## Home Assistant MQTT contract

Topics are `<base_topic>/<node>/…` (defaults `lightmime/<hostname>`):
`availability` (online/offline, retained, LWT), `sync/state|set` (ON/OFF),
`status/state` (`idle|starting|streaming|error`), `status/attributes` (JSON:
area, channels, error), `fps/state` (throttled to every 10 s while streaming),
and `<slider>/state|set` for `brightness`, `smoothing`, `saturation` in percent.
Discovery configs go to `<discovery_prefix>/<component>/lightmime_<node>/<key>/config`
and are re-sent on reconnect and when `homeassistant/status` reports `online`.
Renaming keys or unique_ids creates new entities in Home Assistant, so treat
them as a public API.

## Commands

```sh
cargo build                              # debug build
cargo test                               # unit tests (colour mapping, hex parsing)
cargo clippy --all-targets               # must be warning-free
cargo fmt                                # rustfmt; run before committing
```

System libraries needed to build: GStreamer 1.x dev files (core, app, video),
OpenSSL 3, D-Bus. At runtime also `gst-plugin-pipewire` and an
xdg-desktop-portal backend.

## Testing against real hardware

- `lightmime test` drives the real lights (colour sweep) without screen
  capture; it is the quickest check of pairing, area start/stop and DTLS.
- `lightmime run --start` opens the portal's screen picker on the first run;
  that needs a person at the desktop. The restore token is stored in
  `~/.local/state/lightmime/restore_token`.
- Only one application can stream to an area at a time; stop Lumux or Hue
  Sync first.
- The user's config (`~/.config/lightmime/config.toml`) contains bridge and
  MQTT credentials. Never print it, copy it into the repo or commit it.

## Conventions

- Rust 2024 edition, workspace dependencies declared per crate.
- Errors: `anyhow` with `.context()` describing the failed step; `thiserror`
  only where callers match on variants (`PairError`).
- Logging via `tracing`; the binary honours `RUST_LOG`.
- Comments explain why, not what. Keep doc comments on public items.
- Keep README.md (user-facing) and this file in sync when commands, config
  keys, entities or the crate layout change.
