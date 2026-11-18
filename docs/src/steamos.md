# SteamOS and gamescope

SteamOS has two sessions, and lightmime captures them differently.

## Desktop Mode

Desktop Mode is KDE Plasma, so lightmime works as on any Plasma desktop: capture goes through
`xdg-desktop-portal-kde`, and the tray icon sits in Plasma's panel. The root filesystem is read-only, so install the
[Flatpak](install.md#flatpak) (Discover can open the `.flatpakref`).

## Game Mode

Game Mode runs [gamescope](https://github.com/ValveSoftware/gamescope), which has no screen-cast portal, so the usual
screen picker never appears and capture fails. gamescope does publish its output as a PipeWire stream of its own (Steam
records from it), and lightmime can read that instead:

```toml
# ~/.config/lightmime/config.toml
[capture]
source = "gamescope"
```

There is nothing to choose and nothing to remember: lightmime connects to the PipeWire node named `gamescope`. This
works in any gamescope session, not only on SteamOS.

Game Mode has no tray and no room for a window, so run lightmime as the headless [service](background.md) and start
and stop it from [Home Assistant](home-assistant.md), or with `lightmime start` / `stop` over SSH.

`source = "gamescope"` only works inside gamescope. To use Desktop Mode as well, keep a second config file for Game
Mode with the same `[bridge]` section, and point the service at it:

```sh
lightmime --config ~/.config/lightmime/gamemode.toml run --start
```

The Flatpak reaches PipeWire through its socket for this, as the portal is not involved.

### Not yet tested on a Steam Deck

This source is new and has not been tried on real hardware yet. If it does not work for you, a log from
`RUST_LOG=debug lightmime run --start` and the output of `pw-dump | grep -A3 gamescope` in an
[issue](https://github.com/pataar/lightmime/issues) help most. Known open questions: whether gamescope's stream
always uses the node name `gamescope`, and how HDR output looks.
