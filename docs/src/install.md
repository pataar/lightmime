# Install

| Channel | How |
|---|---|
| Flatpak repository | `flatpak install --user lightmime io.github.pataar.lightmime` after adding the remote, see [below](#flatpak) |
| Flatpak bundle | `lightmime-<arch>.flatpak` from the [latest release](https://github.com/pataar/lightmime/releases/latest) |
| Binary | `lightmime-<arch>-unknown-linux-gnu.tar.xz` from the [latest release](https://github.com/pataar/lightmime/releases/latest), see [below](#binary) |
| From source | `cargo install --path crates/lightmime`, see [below](#from-source) |

Releases are built for x86_64 and aarch64.

## Flatpak

lightmime has its own Flatpak repository, so it installs as a desktop app and `flatpak update` (or your software
centre) brings each new release. The Freedesktop runtime it needs comes from Flathub:

```sh
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak remote-add --user --if-not-exists lightmime https://lightmime.pataar.nl/flatpak/lightmime.flatpakrepo
flatpak install --user lightmime io.github.pataar.lightmime
```

`flatpak run io.github.pataar.lightmime` opens the window; with arguments it is the `lightmime` command, for example
`flatpak run io.github.pataar.lightmime areas`. A shell alias saves typing:

```sh
alias lightmime='flatpak run io.github.pataar.lightmime'
```

The Flatpak keeps its config in `~/.var/app/io.github.pataar.lightmime/config/lightmime/`. It may use the network
(to reach the bridge and, if enabled, the MQTT broker), the display and the GPU. Screen capture needs no extra
permission: the desktop's screen-cast portal asks you which screen to share.

A single release can also be installed from its bundle, without the repository. It does not update itself:

```sh
flatpak install --user lightmime-x86_64.flatpak
```

## Binary

Each release has a tarball with the `lightmime` binary, the licences and the files from `contrib/`. The binary links
against your system's libraries, so install these first:

| Distribution | Packages |
|---|---|
| Arch, CachyOS, Manjaro | `gstreamer gst-plugins-base gst-plugin-pipewire openssl libxkbcommon-x11` |
| Fedora | `gstreamer1 gstreamer1-plugins-base pipewire-gstreamer openssl libxkbcommon-x11` |
| Debian, Ubuntu | `libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-pipewire libssl3 libxkbcommon-x11-0` |

```sh
tar -xJf lightmime-x86_64-unknown-linux-gnu.tar.xz
install -Dm755 lightmime-x86_64-unknown-linux-gnu/lightmime ~/.local/bin/lightmime
```

Then add the [launcher entry and icons](#desktop-integration).

## From source

You need a recent stable Rust (the version is pinned in `rust-toolchain.toml`; rustup picks it up) and the
development files of the libraries above:

| Distribution | Packages |
|---|---|
| Arch, CachyOS, Manjaro | `base-devel gstreamer gst-plugins-base gst-plugin-pipewire openssl libxkbcommon-x11 wayland` |
| Fedora | `gcc pkgconf gstreamer1-devel gstreamer1-plugins-base-devel pipewire-gstreamer openssl-devel libxkbcommon-x11-devel wayland-devel fontconfig-devel` |
| Debian, Ubuntu | `build-essential pkg-config libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev gstreamer1.0-pipewire libssl-dev libxkbcommon-x11-dev libwayland-dev libfontconfig-dev` |

```sh
git clone https://github.com/pataar/lightmime
cd lightmime
cargo install --locked --path crates/lightmime
```

For a machine that only runs headless (no window or tray), leave the GUI out:

```sh
cargo install --locked --path crates/lightmime --no-default-features
```

## Desktop integration

The Flatpak brings its own launcher entry and icons. For a binary or source install, add them yourself so the app
shows up in the launcher and the window and tray get their icons:

```sh
install -Dm644 contrib/io.github.pataar.lightmime.desktop ~/.local/share/applications/io.github.pataar.lightmime.desktop
cp -r assets/icons/hicolor ~/.local/share/icons/
```

## Uninstall

```sh
flatpak uninstall --user io.github.pataar.lightmime   # Flatpak; add --delete-data to remove its config
rm ~/.local/bin/lightmime                             # binary or source install
```

Your settings stay in `~/.config/lightmime/` (or the Flatpak's directory above). The app key lightmime registered on the bridge
stays there; it does no harm.
