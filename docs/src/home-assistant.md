# Home Assistant

lightmime can show up in [Home Assistant](https://www.home-assistant.io) as a device, through
[MQTT discovery](https://www.home-assistant.io/integrations/mqtt/#mqtt-discovery). This is optional and off by
default; without it, lightmime talks only to your bridge.

## Setup

You need an MQTT broker that Home Assistant uses, such as the Mosquitto add-on, and a user for lightmime on it. Add
an `[mqtt]` section to the [config file](configuration.md):

```toml
[mqtt]
enabled = true
host = "homeassistant.local"
port = 1883
username = "lightmime"
password = "change-me"
```

For a broker with TLS, set `port = 8883` and `tls = true`, plus `ca_file` if the broker uses a self-signed
certificate (see [`[mqtt]`](configuration.md#mqtt)).

Then run either the [window](gui.md) (`lightmime gui`) or the [headless daemon](background.md) (`lightmime run`).
Both connect to the broker. Home Assistant finds a device called **lightmime (&lt;hostname&gt;)**.

## Entities

`<node>` is the machine's hostname in lowercase, with anything but letters and digits replaced by `_`
(or `node_id` from the config).

| Entity | What it does |
|---|---|
| `switch.lightmime_<node>_sync` | Starts and stops syncing. |
| `sensor.lightmime_<node>_status` | `idle`, `starting`, `streaming` or `error`. Its attributes hold the area, the number of lights and the error message. |
| `sensor.lightmime_<node>_fps` | Frames per second sent to the bridge, updated every 10 seconds while syncing (diagnostic). |
| `number.lightmime_<node>_brightness` | [Brightness](tuning.md), 0–100 %. |
| `number.lightmime_<node>_smoothing` | [Smoothing](tuning.md), 0–95 %. |
| `number.lightmime_<node>_saturation` | [Saturation](tuning.md), 0–300 %. |

Slider changes from Home Assistant apply right away, show up in the window and are saved to the config file. The
device becomes unavailable when lightmime stops.

## Example: sync while the TV plays

```yaml
automation:
  - alias: Sync the lights with the PC while it plays
    triggers:
      - trigger: state
        entity_id: media_player.living_room_pc
        to: playing
    actions:
      - action: switch.turn_on
        target:
          entity_id: switch.lightmime_htpc_sync
  - alias: Stop syncing when playback stops
    triggers:
      - trigger: state
        entity_id: media_player.living_room_pc
        from: playing
        for: "00:01:00"
    actions:
      - action: switch.turn_off
        target:
          entity_id: switch.lightmime_htpc_sync
```

## Topics

For other MQTT clients: everything lives under `<base_topic>/<node>/` (default `lightmime/<hostname>/`).

| Topic | Payload |
|---|---|
| `availability` | `online` or `offline` (retained; `offline` is also the last will) |
| `sync/state`, `sync/set` | `ON` or `OFF` |
| `status/state` | `idle`, `starting`, `streaming` or `error` |
| `status/attributes` | JSON: `{"area": …, "channels": …, "error": …}` |
| `fps/state` | frames per second |
| `brightness/state`, `brightness/set` (and `smoothing`, `saturation`) | percent |

Discovery configs go to `<discovery_prefix>/<component>/lightmime_<node>/<key>/config`. They are sent again when
lightmime reconnects and when Home Assistant reports `online` on `<discovery_prefix>/status`, so a Home Assistant
restart picks the device up again.
