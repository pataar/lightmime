# CLI reference

```text
lightmime [--config <path>] <command>
```

`--config` works with every command and defaults to `~/.config/lightmime/config.toml` (see
[Configuration](configuration.md)). With the Flatpak, run commands as `flatpak run io.github.pataar.lightmime <command>`;
without a command, that opens the window. `RUST_LOG` sets the log level, for example `RUST_LOG=debug`.

## `lightmime gui`

Open the [window and tray icon](gui.md). Runs the [Home Assistant](home-assistant.md) bridge too when MQTT is enabled.

| Option | |
|---|---|
| `--hidden` | Start in the tray without opening the window. Ignored when there is no tray. |
| `--start` | Start syncing right away. |

Not available in a build without the `gui` feature.

## `lightmime run`

[Sync without a window](background.md), controlled over MQTT when that is enabled. Stops on Ctrl-C or SIGTERM.

| Option | |
|---|---|
| `--start` | Start syncing right away instead of waiting for Home Assistant. |

## `lightmime pair --host <address>`

[Pair](bridge.md#pairing) with the bridge at `<address>`. Press the bridge's link button within a minute.

## `lightmime import-lumux`

Copy the bridge address, keys and entertainment area from Lumux's settings. See
[Reusing Lumux's pairing](bridge.md#reusing-lumuxs-pairing).

## `lightmime areas`

List the bridge's [entertainment areas](bridge.md#entertainment-areas): id, name, number of lights and whether an app
is streaming to it. `*` marks the area lightmime uses.

## `lightmime use-area <area>`

Sync to `<area>`, given by id or by name (case-insensitive).

## `lightmime test`

Run a colour sweep over the area's lights without capturing the screen, to [check the connection](bridge.md#checking-the-connection).

| Option | |
|---|---|
| `--seconds <n>` | How long to run; default 8. |
