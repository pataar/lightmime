# lightmime-core

The engine behind [lightmime](https://lightmime.pataar.nl): screen capture through the XDG ScreenCast portal and
PipeWire, colour mapping per Hue Entertainment channel, and HueStream v2 over DTLS to the bridge. It has no user
interface, so you can put your own frontend on it. The `lightmime` CLI, daemon and window all use this crate's public
API, nothing else.

```rust
use lightmime_core::{Config, Engine};

// Inside a tokio runtime.
let engine = Engine::spawn(config, Some(restore_token_path));
engine.start();
let mut status = engine.subscribe(); // state, fps, area, channels
engine.update_settings(|s| s.brightness = 0.8); // clamped, applies on the next frame
engine.stop(); // also stops the area on the bridge
```

Pairing and area selection are in [`hue`](src/hue/api.rs): `discover()`, `pair()` / `pair_waiting()` and
`Bridge::entertainment_configurations()`. Change the bridge or area of a running engine with `Engine::set_bridge`.
[`examples/minimal.rs`](examples/minimal.rs) does all of it in one file:

```sh
cargo run -p lightmime-core --example minimal -- 192.168.1.20
```

## Things to know

- `Engine::spawn` must run inside a tokio runtime; network calls use reqwest.
- If your crate uses `ashpd`, enable its `async-io` feature, not `tokio`: Cargo merges both into one build and ashpd
  refuses that.
- Persisting the config is up to you. `Config` and its parts are serde types; the restore token path lets the portal
  skip the screen picker after the first run.
- Stop the engine and wait until `status.state.is_active()` is false before exiting, or the lights stay locked in
  streaming mode.
- Only one application can stream to an entertainment area at a time.
- Building needs GStreamer and OpenSSL 3 development files; running needs `gst-plugin-pipewire` and an
  `xdg-desktop-portal` backend with screen casting. Linux only.
- The API is not stable before 1.0; pin a minor version.

## Licence

MIT or Apache-2.0, at your option.
