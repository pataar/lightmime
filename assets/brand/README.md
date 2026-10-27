# lightmime brand

The mark is a screen whose colours spill onto the wall behind it: the lights
mimicking the picture. The wordmark is drawn as monoline strokes (no font
needed), with the two i-dots as the warm and cool ends of the spectrum.

## Files

| File | Use |
| --- | --- |
| `../icons/hicolor/scalable/apps/lightmime.svg` | App icon (256 px tile). Desktop file, window icon, avatar. |
| `../icons/hicolor/symbolic/apps/lightmime-symbolic.svg` | Monochrome 16 px icon for the tray and symbolic contexts. |
| `lightmime-logo.svg` | Icon + wordmark, for light backgrounds. |
| `lightmime-logo-dark.svg` | Icon + wordmark, for dark backgrounds. |
| `lightmime-wordmark.svg` | Wordmark alone, for light backgrounds. |
| `export.sh` | Renders the app icon to PNGs in a `hicolor` icon theme tree (needs `rsvg-convert`). |

## Colours

| Name | Hex | Role |
| --- | --- | --- |
| Ember | `#FF7A3D` | Spectrum start, warm accent |
| Orchid | `#F0479B` | Spectrum |
| Violet | `#7C5CFF` | Spectrum, primary accent in UI |
| Glacier | `#22C7E8` | Spectrum end, cool accent |
| Night | `#0C0D18` | Backgrounds, screen bezel |
| Dusk | `#1C1D33` | Raised surfaces |
| Ink | `#14152A` | Text on light backgrounds |
| Paper | `#F4F2FF` | Text on dark backgrounds |

The spectrum always runs Ember → Orchid → Violet → Glacier, left to right
(stops at 0, 35, 68, 100 %).

## Rules

- Write the name in lowercase: lightmime, also at the start of a sentence.
- Keep clear space around the logo of at least the height of the wordmark's x-height.
- Don't recolour the i-dots or the spectrum, and don't put the logo on busy imagery.
- Use the symbolic icon below 32 px; the full icon loses its glow at that size.
