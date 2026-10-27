# Security policy

lightmime holds the keys to your Hue bridge and, optionally, an MQTT password, so security reports are welcome.

## Reporting a vulnerability

Report it privately through [GitHub's private vulnerability reporting](https://github.com/pataar/lightmime/security/advisories/new).
Do not open a public issue.

Include the version (`lightmime --version`), how you installed it (Flatpak, binary, source), and the steps to
reproduce. Leave out real keys and passwords.

You can expect a first reply within a week. Once a fix is released, the advisory is published with credit to you,
unless you would rather stay anonymous.

## Supported versions

Only the latest release gets security fixes.

## In scope

- Leaking the bridge keys or the MQTT password, for example through logs, error messages or files with loose
  permissions
- A crafted MQTT message or bridge response that crashes lightmime or makes it act on something it should not
- Flatpak permissions broader than lightmime needs

## Known trade-offs

- The bridge serves a certificate signed by Signify's own authority, so lightmime does not verify it on the REST API.
  That traffic stays on your local network. The streaming connection is authenticated with the client key.
- MQTT uses plain TCP. Use a broker on your own network, or put TLS in front of it.
