# Configuration

lightmime keeps its settings in one TOML file. The window and the CLI write it for you; editing it by hand is only
needed for the settings without a control, and for [MQTT](home-assistant.md).

| Install | Config file |
|---|---|
| Native | `~/.config/lightmime/config.toml` |
| Flatpak | `~/.var/app/io.github.pataar.lightmime/config/lightmime/config.toml` |

`--config <path>` points any command at another file. The file holds the bridge keys and the MQTT password, so
lightmime writes it readable by you only (mode 0600). A missing file or key means the default.

Restart lightmime after editing the file by hand; the window and Home Assistant write their own changes into it.

An annotated example is in
[`contrib/config.example.toml`](https://github.com/pataar/lightmime/blob/main/contrib/config.example.toml).

## `[bridge]`

| Key | Default | Meaning |
|---|---|---|
| `host` | | The bridge's IP address or hostname. |
| `app_key` | | Application key from [pairing](bridge.md#pairing). |
| `client_key` | | 32 hex characters from pairing; the key for the encrypted stream. |
| `entertainment_config_id` | | The [area](bridge.md#entertainment-areas) to sync to (set by `lightmime use-area`). |

## `[capture]`

| Key | Default | Meaning |
|---|---|---|
| `source` | `"monitor"` | What the screen picker offers: `"monitor"` or `"window"`. |
| `width`, `height` | `64`, `36` | Frames are scaled down to this size before the colours are taken. More gives no visible gain. |

## `[sync]`

| Key | Default | Range | Meaning |
|---|---|---|---|
| `fps` | `30` | 1–60 | Frames per second sent to the bridge. |
| `brightness` | `1.0` | 0.0–1.0 | Output brightness. |
| `smoothing` | `0.3` | 0.0–0.95 | Smoothing between frames. |
| `saturation` | `1.2` | 0.0–3.0 | Saturation; 1.0 keeps the screen's colours. |
| `region_size` | `0.3` | 0.05–1.0 | Part of the screen each light averages. |

Values outside the range are clamped. See [Tuning](tuning.md).

## `[mqtt]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Connect to the broker and appear in Home Assistant. |
| `host`, `port` | `""`, `1883` | The broker. |
| `username`, `password` | `""` | Broker login, if it needs one. |
| `tls` | `false` | Connect over TLS. Brokers usually listen for TLS on port 8883. |
| `ca_file` | `""` | PEM file with the CA that signed the broker's certificate, for a self-signed broker. Empty trusts the system's CAs. |
| `discovery_prefix` | `"homeassistant"` | Home Assistant's discovery prefix. |
| `base_topic` | `"lightmime"` | First part of every topic. |
| `node_id` | the hostname | Names this machine in topics, entity ids and the MQTT client id. Must be unique per machine on the broker. |

## Other files

| What | Native | Flatpak |
|---|---|---|
| Which screen to capture (the portal's restore token) | `~/.local/state/lightmime/restore_token` | `~/.var/app/io.github.pataar.lightmime/.local/state/lightmime/restore_token` |

Delete the restore token to be asked for the screen again.
