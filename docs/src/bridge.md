# Bridge and areas

## Pairing

lightmime needs an app key from the bridge, and for streaming a client key as well. The bridge hands both out once
its link button has been pressed.

In the window, press **Find** to ask Signify's discovery service (`discovery.meethue.com`) for the bridges on your
network, or type the bridge's IP address; the Hue app shows it under *Settings → Bridges → (your bridge)*. Press
**Pair**, then the link button on the bridge. lightmime keeps trying for a minute.

From the command line:

```sh
lightmime pair --host 192.168.1.20
```

The keys go into the [config file](configuration.md), which is readable by you only. **Forget** in the window removes
them there; the bridge keeps the registration, which does no harm.

### Reusing Lumux's pairing

If [Lumux](https://github.com/enginkirmaci/lumux) (installed as a Flatpak) is already paired, lightmime can copy its
bridge address, keys and entertainment area instead:

```sh
lightmime import-lumux
```

This reads Lumux's settings from `~/.var/app/io.github.enginkirmaci.lumux/`, which the lightmime Flatpak cannot see.
Run it from a native install, or grant read access for the one command:

```sh
flatpak run --filesystem=~/.var/app/io.github.enginkirmaci.lumux/config/lumux:ro io.github.pataar.lightmime import-lumux
```

## Entertainment areas

An entertainment area is a group of lights with a position each, set up in the Hue app under
*Settings → Entertainment areas*. lightmime streams to one area at a time and uses each light's position to decide
which part of the screen it shows:

- left to right in the area is left to right on the screen;
- low to high in the area is bottom to top on the screen;
- front and back are ignored.

The outermost lights mark the edges of the screen. Of all lights, the one furthest from the middle to the left or right
gets that edge, and the one furthest above or below the middle gets the top or bottom edge. The rest are stretched out
by the same amount, so the middle of the area stays the middle of the screen: if the furthest light on the left sits at
-0.8, a light at 0.6 on the right lands three quarters of the way to the right edge. So lights beside, above or
below the screen show the edge of the picture nearest to them, not its middle, wherever they sit in the area. A light
strip behind the TV, split into segments, gets the colours along the edges of the picture, and a lamp to the left of
the couch gets the left side. Place the lights in the Hue app as they really stand around the screen.

Positions stretch at most four times, so lights that all stand close to the middle of the area, or at nearly the same
height, are not spread over the whole screen. The dots on the window's room map show where each light samples the screen.

Choose the area in the window, or:

```sh
lightmime areas                # list them; * marks the current one
lightmime use-area "Living room"
```

`lightmime areas` also shows whether an area is *active*, meaning some app is streaming to it. Only one app can stream
to an area at a time; stop Hue Sync, Lumux or another lightmime first.

## Checking the connection

```sh
lightmime test
```

runs a colour sweep over the lights for eight seconds without capturing the screen. If the lights cycle through the
rainbow, pairing, the area and the streaming connection all work. `--seconds` changes the duration.
