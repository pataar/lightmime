# Troubleshooting

Start with the log. Run lightmime from a terminal (`lightmime gui`, or
`flatpak run io.github.pataar.lightmime gui`) and add `RUST_LOG=debug` for more. A service logs to
`journalctl --user -u lightmime`.

## The lights do nothing

Run `lightmime test`. If the lights don't cycle colours, the problem is between lightmime and the bridge:

- **"not paired"**: pair first, see [Bridge and areas](bridge.md).
- **"bridge refused start"** or an area marked *active*: another app is streaming to the area. Stop Hue Sync, Lumux,
  a game console's sync or another lightmime. After a crash the bridge releases the area by itself within about ten
  seconds.
- **The DTLS handshake fails or times out**: the bridge must be reachable on UDP port 2100. A firewall on the machine
  or a VPN that routes the LAN elsewhere can block it.

If the test works but syncing doesn't, look at the screen capture.

## The screen picker appears every time

The desktop forgot the choice, or its portal does not support restoring one. Choose the screen again; if it keeps
asking, check that `xdg-desktop-portal` and its backend are current. GNOME, KDE Plasma and `xdg-desktop-portal-wlr`
remember the choice.

## "connecting to the ScreenCast portal" or "screen sharing was not allowed"

- No portal backend with screen casting is installed. Install the one for your desktop: `xdg-desktop-portal-kde`,
  `xdg-desktop-portal-gnome`, `xdg-desktop-portal-wlr` (Sway, river), `xdg-desktop-portal-hyprland`.
- You pressed Cancel in the picker. Press Start again.
- lightmime runs outside your graphical session, for example as a system service or over SSH. Run it as a user service
  in the session, see [Running in the background](background.md).

## "is gst-plugin-pipewire installed?"

The GStreamer PipeWire plugin is missing (native installs only; the Flatpak has it). Install `gst-plugin-pipewire`
(Arch), `pipewire-gstreamer` (Fedora) or `gstreamer1.0-pipewire` (Debian, Ubuntu).

## The colours are off, or the wrong lights react

lightmime maps each light by its position in the entertainment area. Check the positions in the Hue app; see
[Entertainment areas](bridge.md#entertainment-areas). If the picture is right but the light is too pale or too
nervous, see [Tuning](tuning.md).

## The lights lag or stutter

- Raise smoothing a little: very low smoothing passes on every jump of the picture.
- PipeWire only sends a new frame when the screen changes; a still picture sends none, which is expected.
- On a busy Wi-Fi network, put the machine or the bridge on a cable.

## There is no tray icon

Your panel does not support StatusNotifierItem. On GNOME, install the AppIndicator extension; see
[Tray](gui.md#tray). Without a tray, closing the window quits lightmime.

## The window shows the wrong font or no text

lightmime uses the first installed font out of Adwaita Sans, Cantarell, Noto Sans, Inter, Ubuntu, IBM Plex Sans,
DejaVu Sans, Liberation Sans and Roboto. Installing any of them fixes it.

## Reporting a bug

Open an issue on [GitHub](https://github.com/pataar/lightmime/issues) with the output of `lightmime --version`, your
distribution and desktop, and the log. Remove the bridge's keys and your MQTT password from anything you paste.
