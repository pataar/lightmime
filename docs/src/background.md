# Running in the background

`lightmime run` syncs without a window or tray. It waits for a command from [Home Assistant](home-assistant.md), or
starts right away with `--start`, and stops on Ctrl-C or SIGTERM, releasing the area on the bridge.

```sh
lightmime run            # wait for Home Assistant (needs [mqtt] enabled)
lightmime run --start    # sync until stopped
```

It still captures the screen, so it has to run inside your graphical session: the screen-cast portal belongs to the
session, and the first start shows its screen picker. After that the choice is remembered and later starts are
silent, which is what makes running at login work.

For a tray icon at login rather than a headless service, use **Start at login** in the [window](gui.md) instead.

## As a systemd user service

The unit in [`contrib/lightmime.service`](https://github.com/pataar/lightmime/blob/main/contrib/lightmime.service)
starts with your graphical session and restarts after a failure.

For a binary or source install in `~/.local/bin`:

```sh
install -Dm644 contrib/lightmime.service ~/.config/systemd/user/lightmime.service
systemctl --user daemon-reload
systemctl --user enable --now lightmime
```

For the Flatpak, write the unit with the Flatpak's command instead:

```ini
# ~/.config/systemd/user/lightmime.service
[Unit]
Description=lightmime: Hue screen sync
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=/usr/bin/flatpak run --command=lightmime io.github.pataar.lightmime run
Restart=on-failure
RestartSec=5

[Install]
WantedBy=graphical-session.target
```

Run `lightmime run --start` once by hand first, so you can answer the screen picker.

Follow the log with `journalctl --user -u lightmime -f`. Set `Environment=RUST_LOG=debug` in the `[Service]` section
for more detail.

Run either the service or the window, not both: only one of them can stream to the area.
