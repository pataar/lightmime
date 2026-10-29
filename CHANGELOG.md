# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/pataar/lightmime/compare/v0.1.4...v0.2.0) - 2026-10-29

### Added

- *(engine)* fade between the lights' own state and sync

## [0.1.4](https://github.com/pataar/lightmime/compare/v0.1.3...v0.1.4) - 2026-10-28

### Other

- *(capture)* ask the compositor for frames at the sync rate ([#8](https://github.com/pataar/lightmime/pull/8))

## [0.1.3](https://github.com/pataar/lightmime/compare/v0.1.2...v0.1.3) - 2026-10-27

### Fixed

- *(mqtt)* stop queueing status publishes while disconnected
- *(engine)* ignore stream failures from a session that already ended

## [0.1.2](https://github.com/pataar/lightmime/compare/v0.1.1...v0.1.2) - 2026-10-27

### Other

- fix build
- formatting

## [0.1.1](https://github.com/pataar/lightmime/compare/v0.1.0...v0.1.1) - 2026-10-27

### Added

- *(gui)* option to start syncing at login

### Fixed

- *(mqtt)* use OpenSSL for MQTT TLS instead of rustls

### Other

- share pairing retry, slider limits and state helpers

### Added

- Screen capture through the ScreenCast portal and PipeWire, mapped onto the lights of a Hue entertainment area and
  streamed over the Entertainment API.
- A window and tray icon (`lightmime gui`): pairing with bridge discovery, area choice, live tuning and start at login.
- A headless mode (`lightmime run`) and a systemd user unit.
- Optional Home Assistant integration over MQTT discovery.
- CLI commands to pair, import Lumux's pairing, list and choose areas, and test the connection.
- A Flatpak, with its own repository at <https://lightmime.pataar.nl/flatpak/lightmime.flatpakrepo>.
- Documentation at <https://lightmime.pataar.nl>.
