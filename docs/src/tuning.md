# Tuning

Three settings change the look while syncing. Set them with the sliders in the [window](gui.md), from
[Home Assistant](home-assistant.md), or in the `[sync]` section of the [config file](configuration.md#sync). Changes
take effect on the next frame and are saved.

| Setting | Range | Default | What it does |
|---|---|---|---|
| Brightness | 0–100 % | 100 % | Scales every light's output. Lower it for a subtler glow in a dark room. |
| Smoothing | 0–95 % | 30 % | Blends each frame with the previous ones. Higher is calmer and hides flicker from fast cuts; lower follows the picture more tightly. |
| Saturation | 0–300 % | 120 % | 100 % keeps the screen's colours. A little more makes lights look closer to the picture, because diffuse light on a wall washes colours out. 0 % gives white light that follows only the brightness. |

Three more settings are only in the config file:

| Setting | Range | Default | What it does |
|---|---|---|---|
| `fps` | 1–60 | 30 | Frames per second sent to the bridge. The bridge passes on about 25 updates a second to Zigbee lights, so more rarely helps. |
| `region_size` | 0.05–1.0 | 0.3 | How much of the screen each light averages, as a fraction of the width and height around its position. Smaller is more precise, larger is calmer. |
| `black_bars` | 0.0–0.4 | 0.25 | Black bars of letterboxed or pillarboxed video, up to this fraction of each edge, are left out, so the outer lights show the picture instead of the bars. 0 turns it off. |

## Tips

- For films, try smoothing at 40–60 % and saturation at 130–150 %. For games, smoothing at 10–20 %.
- The outermost lights in the area show the edges of the screen; see
  [Entertainment areas](bridge.md#entertainment-areas). A light that shows too much of the middle of the picture
  sits closer to the middle than the light furthest out on that axis (left and right count together, as do top
  and bottom); move it outward in the Hue app.
- Black bars, at the top and bottom of a wide film or at the sides of a 16:9 video on an ultrawide screen, are left
  out of the picture after they have stayed about two seconds. Bars thicker than `black_bars` stay in, so a scene
  that is mostly dark is not mistaken for bars.
