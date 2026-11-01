# AGENTS.md

Guidance for coding agents working in this repository. User-facing docs are
the mdbook in [docs/src/](docs/src) (published at https://lightmime.pataar.nl/docs/,
behind the landing page in [docs/site/](docs/site));
[README.md](README.md) is the short version.

## What this is

lightmime syncs Philips Hue lights with the screen on Linux. It captures the
screen via the XDG ScreenCast portal + PipeWire, maps it to one colour per Hue
Entertainment channel, and streams those to the bridge over DTLS. Frontends:
a gpui window + tray (`lightmime gui`), a headless daemon (`lightmime run`) and
the CLI, which also drives a running `gui`/`run` over D-Bus (`start`, `stop`,
`toggle`, `show`, `status`; for keybindings and Waybar). Home Assistant over MQTT discovery is optional (`[mqtt] enabled`) and
runs inside either `gui` or `run`. Wayland is the target; X11 works because
gpui and the portal support it, but nothing is X11-specific. No Windows/macOS.

## Layout

```
crates/lightmime-core/     library, no UI, no MQTT
  src/config.rs            Config, BridgeConfig, CaptureConfig, SyncSettings (serde, defaults, sanitized())
  src/capture.rs           portal session + GStreamer pipeline (pipewiresrc → appsink), FrameSource
  src/color.rs             Frame, Zone (channel position → screen point), ColorMapper (sample, saturate, smooth, brightness)
  src/hue/api.rs           CLIP v2 REST: pair(), discover() (discovery.meethue.com), Bridge (areas, start/stop streaming, PSK identity)
  src/hue/stream.rs        EntertainmentStream: DTLS 1.2 PSK + HueStream v2 messages (blocking)
  src/engine.rs            Engine handle + actor; owns capture and bridge session, runs the stream thread
  examples/minimal.rs      smallest frontend: pair, first area, sync until Ctrl-C (keep it building)
crates/lightmime/          the `lightmime` binary
  src/main.rs              CLI: pair, import-lumux, areas, use-area, test, run, gui (feature `gui`, default),
                           start, stop, toggle, show, status (remote control of a running instance)
  src/control.rs           D-Bus: owns the app id on the session bus (single instance), serves Control, CLI client
  src/settings.rs          AppConfig = core Config (flattened) + [mqtt]; load/save (0600, atomic); FileStore for the GUI
  src/mqtt.rs              Home Assistant MQTT discovery bridge
crates/lightmime-gui/      window + tray; library called by `lightmime gui`, no config file or MQTT knowledge
  src/lib.rs               run(Options), Store trait (persistence is the binary's job), app-level window/tray wiring
  src/view.rs              MainView: status, pairing, areas, sliders, start (and sync) at login
  src/widgets.rs           button, TextInput (minimal), slider, switch; gpui has none built in
  src/decorations.rs       titlebar, border and resize margin when the compositor wants client-side decorations (GNOME)
  src/theme.rs             brand palette light/dark, UiFont (picks an installed font), ForceAppearance
  src/tray.rs              ksni StatusNotifierItem; Show/Quit go to gpui over a channel, start/stop to the Engine
  src/autostart.rs         Background portal in Flatpak, ~/.config/autostart file otherwise
  src/icons.rs             APP_ID, icon names, window_options(), tray icon theme path
  src/screenshots.rs       #[ignore] test rendering docs/src/images/window-{light,dark}.png headless
contrib/                   example config, systemd user unit, .desktop file (single source; Flatpak rewrites Icon=)
assets/brand/              logo, wordmark, palette (README.md), export.sh → hicolor PNGs
assets/icons/hicolor/      app icon `lightmime` (scalable + pixel-aligned 16/24 px drawings) and tray icon `lightmime-symbolic` (SVG)
docs/                      Pages site: build.sh assembles site/ (landing page, CNAME, robots.txt) at /, the mdbook
                           (book.toml, src/, theme/) at /docs/, a sitemap and redirects from old URLs; also the Flatpak repo
packaging/aur/             PKGBUILD for `lightmime-bin`, update.sh (version + checksums), publish.sh (push to the AUR);
                           not on the AUR yet (registration closed), so the docs do not mention it
packaging/linux/           metainfo, Flatpak manifest + launcher, flatpak.sh (bundle), flatpak-repo.sh (site repo), apt build deps
.github/workflows/         ci.yml, docs.yml (Pages), release.yml (tag → binaries + Flatpak → GitHub release),
                           dependabot-auto-merge.yml (non-major updates squash-merge once `check` passes)
```

## Architecture rules

- `lightmime-core` stays frontend-agnostic: no MQTT, CLI or GUI code there.
  Frontends interact only through `Engine` (`spawn`, `start`, `stop`,
  `set_bridge`, `status`/`subscribe`, `settings`/`subscribe_settings`/`update_settings`,
  `subscribe_colors`; the window draws its room map and background from the latter while syncing).
  `lightmime-gui` embeds `Engine` the same way the daemon does; with MQTT on,
  both share one engine in the `gui` process.
- The engine is an actor on the tokio runtime; frame streaming runs on a
  dedicated std thread (`StreamWorker`) because DTLS I/O is blocking and the
  loop is time-critical. Live settings live in a `tokio::sync::watch` channel
  (read every frame with `borrow()`, so the window and MQTT see each other's
  changes); status goes out through another `watch` channel.
- `set_bridge` replaces the bridge config (after pairing or choosing an area
  in the GUI) and restarts a running session on it.
- Stream errors are reported back to the actor (`Command::StreamFailed`), which
  stops the entertainment area and capture and moves to `EngineState::Error`.
  Always stop the area on the bridge when a session ends, or the lights stay
  locked in streaming mode.
- `SyncSettings::sanitized()` is the single place values get clamped; call it
  whenever settings change.

- The GUI's window icon comes from the desktop file, matched by app id
  (`icons::APP_ID` = `io.github.pataar.lightmime`, the `.desktop` file name and
  `StartupWMClass`). Changing one means changing all three, plus the Flatpak id.
  Inside the Flatpak, icons must be named after the app id, so `flatpak.sh`
  renames `lightmime*` icons and the tray asks for `<APP_ID>-symbolic` there.

## GUI notes

- gpui comes from `gpui-pre` / `gpui-pre-platform` (crates.io snapshots of
  Zed's gpui, renamed to `gpui` / `gpui_platform` in Cargo.toml). The crates.io
  `gpui` 0.2 does not build against current libc. Bump all `gpui-pre*` together.
- gpui runs on the main thread (`block_in_place` in `main.rs`); network calls
  from the window go to the engine's tokio runtime (`MainView::run_job`),
  because reqwest needs tokio. tokio `watch`/`mpsc` receivers are awaited from
  gpui tasks directly.
- `ashpd` must use the `async-io` feature everywhere: gpui enables it, and
  ashpd refuses to build with both `async-io` and `tokio`.
- On GNOME Wayland Mutter offers no server-side decorations and gpui draws
  none, so `decorations::frame` draws the titlebar itself. It is a no-op when
  the window manager decorates (Plasma, X11); the headless screenshots always
  take that path, so they do not show the GNOME titlebar.
- Closing the window keeps the app in the tray; without a tray (no
  StatusNotifierWatcher) it quits. SIGINT/SIGTERM quit like the tray's Quit,
  so the area is always released.
- After visible changes run `cargo test -p lightmime-gui screenshots -- --ignored`
  and commit the regenerated docs images.

## Protocol notes

- Pairing: `POST /api` with `generateclientkey: true` returns `username`
  (app key) and `clientkey` (32 hex chars = the PSK). Error type 101 means the
  link button was not pressed.
- Streaming: `PUT /clip/v2/resource/entertainment_configuration/{id}` with
  `{"action":"start"}` first, then DTLS to UDP 2100, cipher
  `PSK-AES128-GCM-SHA256`. PSK identity is the `hue-application-id` header from
  `GET /auth/v1`, falling back to the app key.
- Message: `HueStream`, version `02 00`, sequence, `00 00`, colour space
  (`00` = RGB), `00`, 36-byte area UUID, then per channel `id` + 3 × u16 BE.
  Max 20 channels. One `ssl_write` = one datagram (see `Datagram` in
  `stream.rs`); never buffer several messages into one write.
- The bridge drops the session after ~10 s without data, so the loop sends
  the lights' own state (read over REST before `start`) until the first frame
  arrives, then wipes into sync left to right; on stop it wipes back before
  closing, so the stop takes ~1.2 s longer.
- Channel positions: x = left/right, y = front/back (ignored), z = bottom/top,
  all in -1..1. `Zone::from_channel` maps x and z to screen coordinates.
- The bridge uses a certificate from Signify's own CA; the REST client accepts
  invalid certs on purpose (LAN only).

## Home Assistant MQTT contract

Topics are `<base_topic>/<node>/…` (defaults `lightmime/<hostname>`):
`availability` (online/offline, retained, LWT), `sync/state|set` (ON/OFF),
`status/state` (`idle|starting|streaming|error`), `status/attributes` (JSON:
area, channels, error), `fps/state` (throttled to every 10 s while streaming),
and `<slider>/state|set` for `brightness`, `smoothing`, `saturation` in percent.
Discovery configs go to `<discovery_prefix>/<component>/lightmime_<node>/<key>/config`
and are re-sent on reconnect and when `homeassistant/status` reports `online`.
Renaming keys or unique_ids creates new entities in Home Assistant, so treat
them as a public API.

## D-Bus contract

`lightmime gui` and `lightmime run` own `io.github.pataar.lightmime` (the app id,
which a Flatpak may own without a permission) on the session bus and serve
interface `io.github.pataar.lightmime1` at `/io/github/pataar/lightmime`:
methods `Start`, `Stop`, `Toggle`, `Show` (NotSupported under `run`); properties
`State` (`idle|starting|streaming|error`), `Error`, `Area`, `Channels` (all emit
PropertiesChanged) and `Fps` (does not). A second `gui` hands over to the owner
(`Show`, plus `Start` with `--start`) and exits; a second `run` fails. Users bind
keys and Waybar modules to these, so treat names and states as a public API;
make an incompatible change as a new `…lightmime2` interface. `lightmime status
--waybar` prints `{"text","alt","class","tooltip"}` with the state (`off` when
nothing owns the name) in `text`, `alt` and `class`.

## Commands

`mise install` brings codegraph and mdbook. Run `codegraph init -y` once per
clone and per worktree; a worktree inside this one otherwise reads the main
checkout's index, which holds another branch. It re-indexes on its own after
that; `.mcp.json` registers its MCP server.

```sh
cargo build                              # debug build
cargo test --all-features                # unit tests (colour mapping, hex parsing)
cargo clippy --all-targets --all-features -- -D warnings          # must be warning-free
cargo clippy -p lightmime --no-default-features -- -D warnings    # headless build
cargo fmt                                # rustfmt; run before committing
cargo test -p lightmime-gui screenshots -- --ignored              # docs screenshots (GPU)
mdbook serve docs                        # preview the docs
docs/build.sh                            # whole site (landing + docs) into docs/book
FLATPAK_BUILDER="flatpak run org.flatpak.Builder" \
  packaging/linux/flatpak.sh target/release/lightmime x86_64 target/lightmime.flatpak
```

System libraries needed to build: see `packaging/linux/build-deps-apt.txt`
(GStreamer core/base dev files, OpenSSL 3, xkbcommon, xcb, wayland). At
runtime also `gst-plugin-pipewire` and an xdg-desktop-portal backend.

## Releasing

release-plz (`release-plz.toml`, `.github/workflows/release-plz.yml`) keeps a
release PR open that bumps the shared version and writes CHANGELOG.md from the
Conventional Commit titles. Merging it makes release-plz publish the three
crates to crates.io (core, gui, then the binary; trusted publishing via OIDC)
and push the `vX.Y.Z` tag (with `RELEASE_PLZ_TOKEN`, so the tag starts workflows). `release.yml` checks
the tag matches, builds the tarballs and Flatpak bundles (x86_64, aarch64),
creates the GitHub release and triggers `docs.yml`, which imports the bundles
into the Flatpak repo on the site (signed when the `FLATPAK_GPG_PRIVATE_KEY`
secret is set). Its `aur` job then pushes `packaging/aur/PKGBUILD`, with the
version and checksums filled in by `update.sh`, to the AUR as `lightmime-bin`
(only for stable tags, and only when the `AUR_SSH_PRIVATE_KEY` secret is set; it
is not yet, as AUR registration is closed, so the job skips itself). Once the
package exists, add it to docs/src/install.md, docs/src/hyprland.md and README.md.
The PKGBUILD in the repo is the template; change packaging there. A
packaging-only fix needs a `pkgrel` bump pushed by hand with `publish.sh`.

## Testing against real hardware

- `lightmime test` drives the real lights (colour sweep) without screen
  capture; it is the quickest check of pairing, area start/stop and DTLS.
- `lightmime run --start` opens the portal's screen picker on the first run;
  that needs a person at the desktop. Don't open windows on the user's desktop
  to check the GUI; render it with the screenshots test instead. The restore token is stored in
  `~/.local/state/lightmime/restore_token`.
- Only one application can stream to an area at a time; stop Lumux or Hue
  Sync first.
- The user's config (`~/.config/lightmime/config.toml`) contains bridge and
  MQTT credentials. Never print it, copy it into the repo or commit it.

## Conventions

- Rust 2024 edition, workspace dependencies declared per crate.
- Errors: `anyhow` with `.context()` describing the failed step; `thiserror`
  only where callers match on variants (`PairError`).
- Logging via `tracing`; the binary honours `RUST_LOG`.
- Comments explain why, not what. Keep doc comments on public items.
- Keep docs/src, docs/site, README.md and this file in sync when commands, config keys,
  entities, the D-Bus interface, the window or the crate layout change.
