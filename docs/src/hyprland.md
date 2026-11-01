# Hyprland and Omarchy

lightmime works on Hyprland as it does on any Wayland desktop: capture goes through `xdg-desktop-portal-hyprland`, the
tray icon shows up in Waybar's `tray` module, and Hyprland draws no titlebars, so neither does lightmime. On a tiling
desktop you mostly drive it from the keyboard and the bar, so this page covers a keybinding, a Waybar module, a window
rule and starting at login. [Omarchy](https://omarchy.org) is Hyprland with Waybar, so all of it applies there; the
file names below are Omarchy's.

The snippets work on any desktop, not just Hyprland: they use `lightmime toggle` and `lightmime status`, which talk to
the running lightmime over the session bus (see [Controlling a running lightmime](#controlling-a-running-lightmime)).

## Install

Any of the ways on [Install](install.md) works. The snippets below call `lightmime`, as the
[binary](install.md#binary) and [source](install.md#from-source) installs put it on your `PATH`. With the
[Flatpak](install.md#flatpak), write `flatpak run io.github.pataar.lightmime` in its place: keybindings and Waybar
do not see shell aliases.

You also need a screen-cast portal backend; Omarchy has `xdg-desktop-portal-hyprland` already.

## Toggle sync with a key

Hyprland 0.53 and 0.54 (`hyprland.conf`; on Omarchy, `~/.config/hypr/bindings.conf`):

```ini
bindd = SUPER SHIFT, L, Toggle light sync, exec, lightmime toggle
```

Hyprland 0.55 and later (`hyprland.lua`):

```lua
hl.bind("SUPER + SHIFT + L", hl.dsp.exec_cmd("lightmime toggle"))
```

`lightmime toggle` needs lightmime to be running, as the window/tray (`lightmime gui`) or as the
[service](background.md) (`lightmime run`); see [Start at login](#start-at-login).

## Waybar module

`lightmime status --waybar --follow` prints a line of JSON every time the state changes: `off` (lightmime is not
running), `idle`, `starting`, `streaming` or `error`. The state is the line's `alt` and `class`, so icons and CSS can key
on it. Add the module to `~/.config/waybar/config.jsonc` and list `"custom/lightmime"` in one of `modules-left`,
`modules-center` or `modules-right`:

```jsonc
"custom/lightmime": {
  "exec": "lightmime status --waybar --follow",
  "return-type": "json",
  "format": "{icon}",
  "format-icons": {
    "off": "",
    "idle": "󰌶",       // nf-md-lightbulb_outline
    "starting": "󰌵",   // nf-md-lightbulb
    "streaming": "󰛨",  // nf-md-lightbulb_on
    "error": "󰹏"       // nf-md-lightbulb_off
  },
  "on-click": "lightmime toggle",
  "on-click-right": "lightmime show",
  "tooltip": true
}
```

The icons are Nerd Font glyphs, which Omarchy's bar font has; use any text you like instead. `off` maps to nothing, so
the module disappears while lightmime is not running. To colour the icon while syncing, in
`~/.config/waybar/style.css`:

```css
#custom-lightmime.streaming { color: #f5c06b; }
#custom-lightmime.error { color: #e06c75; }
```

The module keeps one `lightmime status` process running, which waits on the session bus and uses no CPU between
changes; it carries on when lightmime quits and starts again.

## Float the window

The window is a small settings panel; tiled, it gets stretched over half the screen. Float it at its own size instead.

Hyprland 0.53 and 0.54 (on Omarchy, `~/.config/hypr/hyprland.conf` or a file it sources):

```ini
windowrule = match:class io.github.pataar.lightmime, float on
windowrule = match:class io.github.pataar.lightmime, size 440 760
windowrule = match:class io.github.pataar.lightmime, center on
```

Hyprland 0.55 and later:

```lua
hl.window_rule({
  match = { class = "io.github.pataar.lightmime" },
  float = true,
  size = { 440, 760 },
  center = true,
})
```

## Start at login

Either of these; not both, as only one lightmime runs at a time.

- **The tray icon:** turn on **Start at login** in the [window](gui.md). It writes an entry to
  `~/.config/autostart`, which Omarchy's session (uwsm) starts. On a Hyprland setup without uwsm, autostart entries
  are not run; add this to `hyprland.conf` instead (on Omarchy, `~/.config/hypr/autostart.conf` uses
  `uwsm-app -- lightmime gui --hidden`):

  ```ini
  exec-once = lightmime gui --hidden
  ```

- **Headless, as a systemd user service:** see [Running in the background](background.md). `lightmime show` has no
  window to open then.

Either way, start lightmime by hand once first, to answer the screen picker; after that it starts silently.

## Controlling a running lightmime

`lightmime gui` and `lightmime run` own the name `io.github.pataar.lightmime` on the session bus. The commands
`lightmime start`, `stop`, `toggle`, `show` and `status` talk to whichever one is running; see the
[CLI reference](cli.md#controlling-a-running-lightmime). Owning the name also keeps a second copy from starting: a
second `lightmime gui` (from the app launcher, say) brings the running window forward instead, and a second
`lightmime run` exits with an error.

Other tools can use the interface directly:

| Member | |
|---|---|
| `Start()`, `Stop()`, `Toggle()` | As the commands. |
| `Show()` | Open the window; fails with `org.freedesktop.DBus.Error.NotSupported` under `lightmime run`. |
| `State` (s) | `idle`, `starting`, `streaming` or `error`. |
| `Error` (s) | Why syncing stopped, while `State` is `error`. |
| `Area` (s), `Channels` (u) | The entertainment area being synced. |
| `Fps` (d) | Frames sent per second. Read on request; not signalled. |

The object is `/io/github/pataar/lightmime`, the interface `io.github.pataar.lightmime1`. Every property except `Fps`
emits `PropertiesChanged`. For example:

```sh
busctl --user call io.github.pataar.lightmime /io/github/pataar/lightmime io.github.pataar.lightmime1 Toggle
busctl --user get-property io.github.pataar.lightmime /io/github/pataar/lightmime io.github.pataar.lightmime1 State
```
