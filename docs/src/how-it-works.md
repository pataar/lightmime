# How it works

lightmime is a pipeline of three stages, run every frame.

## 1. Capture

The [XDG ScreenCast portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
asks you which screen to share and hands lightmime a PipeWire stream of it, with a restore token so it can skip the
question next time. GStreamer receives the stream and scales it down to 64 × 36 pixels of RGB:

```text
pipewiresrc ! videoconvert ! videoscale ! video/x-raw,format=RGB,width=64,height=36 ! appsink
```

The portal is the only way to capture the screen on Wayland, and it works the same in a Flatpak, so lightmime needs no
special permissions.

## 2. Mapping

Each light in the [entertainment area](bridge.md#entertainment-areas) has a position. Its left-right coordinate becomes
a horizontal position on the screen and its height a vertical one. lightmime averages the pixels in a square around
that point (`region_size` of the screen), in linear light so that bright and dark parts mix the way light does. Then
it applies [saturation, smoothing and brightness](tuning.md).

## 3. Streaming

The bridge's [Entertainment API](https://developers.meethue.com/develop/hue-entertainment/) takes colours over an
encrypted UDP stream:

1. lightmime starts the area with a `PUT` on `/clip/v2/resource/entertainment_configuration/<id>`.
2. It opens a DTLS 1.2 connection to UDP port 2100, authenticated with the client key from pairing
   (`PSK-AES128-GCM-SHA256`).
3. Every frame goes out as one `HueStream` v2 message: a header, the area's id and, per light, its channel and a
   16-bit red, green and blue value.
4. On stop, it ends the connection and stops the area, so the lights return to normal.

## Code

The repository is a Cargo workspace:

| Crate | |
|---|---|
| `lightmime-core` | Capture, colour mapping, the Hue API and streaming, and the `Engine` that runs them. No user interface. |
| `lightmime-gui` | The window and tray icon, built with [gpui](https://gpui.rs). |
| `lightmime` | The binary: the CLI, the headless daemon and the Home Assistant bridge. |

See [AGENTS.md](https://github.com/pataar/lightmime/blob/main/AGENTS.md) for the architecture and protocol details.
