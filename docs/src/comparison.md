# Compared with other tools

## Resource use

lightmime is one native binary with no interpreter, browser engine or web server. The compositor copies the screen
into PipeWire; lightmime drops frames above the sync rate and scales each one down to 64 × 36 pixels before converting
it, so colour mapping only ever looks at 2,304 pixels a frame. See [How it works](how-it-works.md).

Measured with `lightmime run` syncing a 3440 × 1440 desktop to a four-light area at 30 fps, with the
[Home Assistant](home-assistant.md) bridge on, on an AMD Ryzen 9 5900XT, over 60 seconds:

| | |
|---|---|
| CPU | about 1.5 % of one core (0.7 % user, 0.7 % system) |
| Memory | 36 MB resident, 27 MB proportional set size |
| Binary | 12.5 MB headless, 40 MB with the window and tray (stripped) |

To check it on your own machine while syncing:

```sh
pid=$(pgrep -x lightmime)
grep VmRSS /proc/$pid/status
top -p $pid
```

Lowering `fps` in [Tuning](tuning.md) lowers CPU use further.

## Lumux and HyperHDR

[Lumux](https://github.com/enginkirmaci/lumux) and [HyperHDR](https://github.com/awawa-dev/HyperHDR) are the other
open-source ways to sync Hue lights with a Linux screen.

| | lightmime | Lumux | HyperHDR |
|---|---|---|---|
| Written in | Rust | Python, GTK4 / libadwaita | C++ |
| Lights | Philips Hue | Philips Hue | Hue, LIFX, WLED, LED strips over serial or SPI, and more |
| Screen capture on Wayland | Portal + PipeWire | Portal | Portal + PipeWire, also USB HDMI grabbers |
| Interface | Window, tray icon, CLI | Window, tray icon | Web interface |
| Runs headless | Yes, also as a [systemd user service](background.md) | No | Yes, but PipeWire capture needs it to run as a user application, not a system service |
| Home Assistant | MQTT discovery | No | MQTT |
| Platforms | Linux | Linux | Linux, Windows, macOS |
| Install | Flatpak, binaries | Flathub, pip | Packages, binaries |
| Licence | MIT or Apache-2.0 | MIT | MIT |

**Lumux** is the closest in scope: a GTK desktop app for Hue entertainment areas on Wayland, on Flathub. It
processes frames in Python; its README suggests lowering the capture resolution and frame rate if CPU use is high.

**HyperHDR** is an ambient-lighting server for many kinds of lights and capture sources, with HDR tone mapping, an
LED layout editor and a web interface. If you drive LED strips, capture a TV through an HDMI grabber or need HDR
handling, use HyperHDR; it does far more than lightmime.

lightmime does one thing: Hue sync from a Linux desktop, with little setup and as little overhead as possible.
