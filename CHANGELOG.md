# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/pataar/lightmime/compare/v0.1.0...v0.1.1) - 2026-10-27

### Added

- *(gui)* draw a titlebar when the compositor wants client-side decorations
- *(gui)* option to start syncing at login

### Fixed

- *(config)* keep comments and unknown keys when saving
- *(mqtt)* use OpenSSL for MQTT TLS instead of rustls

### Other

- share pairing retry, slider limits and state helpers
- *(core)* drop unused deps and the second TLS stack

### Added

- Screen capture through the ScreenCast portal and PipeWire, mapped onto the lights of a Hue entertainment area and
  streamed over the Entertainment API.
- A window and tray icon (`lightmime gui`): pairing with bridge discovery, area choice, live tuning and start at login.
- A headless mode (`lightmime run`) and a systemd user unit.
- Optional Home Assistant integration over MQTT discovery.
- CLI commands to pair, import Lumux's pairing, list and choose areas, and test the connection.
- A Flatpak, with its own repository at <https://lightmime.pataar.nl/flatpak/lightmime.flatpakrepo>.
- Documentation at <https://lightmime.pataar.nl>.
