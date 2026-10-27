# Quickstart

1. **Install lightmime.** The Flatpak is the easiest way:

   ```sh
   flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
   flatpak remote-add --user --if-not-exists lightmime https://lightmime.pataar.nl/flatpak/lightmime.flatpakrepo
   flatpak install --user lightmime io.github.pataar.lightmime
   ```

   Other ways are under [Install](install.md).

2. **Create an entertainment area** in the Hue app if you have none yet: *Settings → Entertainment areas*. Place the
   lights roughly where they stand around your screen; lightmime uses those positions. See
   [Entertainment areas](bridge.md#entertainment-areas).

3. **Open lightmime** from your app launcher, or run `flatpak run io.github.pataar.lightmime`.

4. **Pair with the bridge.** Press **Find** to look the bridge up, or type its IP address. Press **Pair**, then the
   round link button on top of the bridge within a minute.

5. **Choose the area** to sync to.

6. **Press Start.** The first time, your desktop asks which screen to share; pick the one the lights surround. The
   choice is remembered, so later starts are silent.

From then on, the tray icon starts and stops syncing. Closing the window keeps lightmime in the tray; quit from the
tray menu. To have it ready after you log in, turn on **Start at login**.

Prefer a terminal? The same steps with the [CLI](cli.md):

```sh
lightmime pair --host 192.168.1.20   # then press the link button
lightmime areas                      # list the entertainment areas
lightmime use-area "Living room"
lightmime test                       # an 8-second colour sweep, to check the connection
lightmime run --start                # sync until Ctrl-C
```

With the Flatpak, run these as `flatpak run io.github.pataar.lightmime pair --host …` and so on.
