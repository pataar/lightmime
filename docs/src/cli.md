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

When lightmime is already running, this opens that one's window (and starts syncing with `--start`) instead of
starting a second copy. Not available in a build without the `gui` feature.

## `lightmime run`

[Sync without a window](background.md), controlled over MQTT when that is enabled. Stops on Ctrl-C or SIGTERM.

| Option | |
|---|---|
| `--start` | Start syncing right away instead of waiting for Home Assistant. |

Exits with an error when lightmime is already running.

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

## Controlling a running lightmime

These talk to the lightmime that is running, as `lightmime gui` or `lightmime run`, over the session bus. They need no
config file and fail with an error when lightmime is not running. They are meant for keybindings and status bars; see
[Hyprland and Omarchy](hyprland.md) for examples and the D-Bus interface behind them.

| Command | |
|---|---|
| `lightmime start` | Start syncing. |
| `lightmime stop` | Stop syncing and release the area. |
| `lightmime toggle` | Stop when syncing or starting, start otherwise. |
| `lightmime show` | Open the window, or bring it to the front. Fails under `lightmime run`. |
| `lightmime status` | Print the state in a sentence; `lightmime is not running` (and success) when it is not. |

| `status` option | |
|---|---|
| `--waybar` | Print a line of JSON for a [Waybar custom module](hyprland.md#waybar-module) instead. |
| `--follow` | Keep running and print again on every change, also while lightmime is not running. |
