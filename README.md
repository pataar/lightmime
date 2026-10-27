# lightmime

Sync Philips Hue lights with your screen on Linux, controlled from Home Assistant.

lightmime captures the screen through the XDG ScreenCast portal (Wayland and
X11, via PipeWire), turns it into one colour per light and streams those to a
Hue Entertainment area over the bridge's DTLS streaming API, the same protocol
Hue Sync uses. It runs as a small daemon that shows up in Home Assistant over
MQTT discovery, so syncing can be started from a dashboard or an automation.

## Layout

| Crate | What it is |
| --- | --- |
| `crates/lightmime-core` | Library: capture, colour mapping, Hue REST + DTLS streaming, and the `Engine` that ties them together. No UI. |
| `crates/lightmime` | The `lightmime` binary: CLI for pairing and testing, and the daemon with the Home Assistant MQTT bridge. |

A GUI can be added as another crate that embeds `lightmime_core::Engine` (start,
stop, live settings and a status stream), exactly like the daemon does.

## Requirements

- A Hue bridge with an entertainment area set up in the Hue app
- PipeWire and `xdg-desktop-portal` with a backend for your desktop (KDE, GNOME, wlroots, ...)
- GStreamer 1.x with the PipeWire plugin (`gst-plugin-pipewire`) and base plugins
- OpenSSL 3 (DTLS-PSK)

Build: `cargo build --release`

## Setup

```sh
# Pair with the bridge (press its link button when asked)…
lightmime pair --host 192.168.1.20
# …or reuse the credentials Lumux already has
lightmime import-lumux

lightmime areas              # list entertainment areas
lightmime use-area "TV area" # pick one
lightmime test               # 8-second colour sweep, no screen capture
lightmime run --start        # capture and sync until Ctrl-C
```

The first sync asks which screen to share. The choice is remembered (portal
restore token in `~/.local/state/lightmime/`), so later runs start silently.

Config lives in `~/.config/lightmime/config.toml` (mode 0600, it holds
credentials). See [`contrib/config.example.toml`](contrib/config.example.toml).

## Home Assistant

Enable the `[mqtt]` section and run the daemon (`lightmime run`, or the
systemd user unit in [`contrib/`](contrib/lightmime.service)). Home Assistant
discovers a device `lightmime (<hostname>)` with:

| Entity | Purpose |
| --- | --- |
| `switch.…_sync` | Start / stop syncing |
| `sensor.…_status` | `idle`, `starting`, `streaming` or `error`; attributes hold the area and the error message |
| `sensor.…_frame_rate` | Frames per second sent to the bridge (diagnostic) |
| `number.…_brightness`, `…_smoothing`, `…_saturation` | Live tuning; changes are saved to the config |

The device goes unavailable when the daemon stops.

## Systemd user service

```sh
install -Dm755 target/release/lightmime ~/.local/bin/lightmime
install -Dm644 contrib/lightmime.service ~/.config/systemd/user/lightmime.service
systemctl --user daemon-reload
systemctl --user enable --now lightmime
```

The service runs inside your graphical session, which the screen-cast portal needs.

## How it works

1. **Capture**: ScreenCast portal → PipeWire node → GStreamer
   (`pipewiresrc ! videoconvert ! videoscale`) → 64×36 RGB frames.
2. **Mapping**: each entertainment channel samples the screen region matching
   its position in the area (x = left/right, z = bottom/top), averaged in
   linear light, then saturation, smoothing and brightness are applied.
3. **Streaming**: the area is started through the CLIP v2 API, then HueStream
   v2 messages go out over DTLS 1.2 (PSK, `PSK-AES128-GCM-SHA256`) to UDP port
   2100, one datagram per frame.

## Development

See [AGENTS.md](AGENTS.md) for the architecture, protocol notes, the MQTT
contract and the build/test commands.
