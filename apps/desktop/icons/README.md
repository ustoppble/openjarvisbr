# OpenJarvisBR icons

`source.svg` is the editable 1024 × 1024 master. It interprets the **Singularity
Three.js design** in `docs/design/openjarvisbr-surreal.html`: an organic energy
core, flowing contour lines, tilted orbital arcs, and floating wireframe
crystals. Laschuk requested this direction during JRV-49, replacing the earlier
seven-dot proposal from `docs/design/referencias.md`.

Cyan `#5fe9ff`, orange `#ff7537`, blue `#267dff`, and violet `#a76dff` match the
Three.js scene. The tile stays OLED-dark `#0c0c0c`. Its transparent outer margin
preserves the rounded silhouette in the macOS Dock. The mark has no lettering.
Contour paths use the same value-noise field as the shader, sampled onto the
front of the orb and converted to continuous SVG paths with marching squares.

`source.png` is its RGBA raster export. `app-icon-source.png` remains an
identical copy for compatibility with the original source filename.

Regenerate the platform assets from `apps/desktop`:

```sh
npx tauri icon icons/source.png
```

The SVGs were rasterized using Chromium at their exact output dimensions,
with a transparent page background and no external assets or fonts.

## Tray states

Each `tray-STATE.svg` has a 22 × 22 viewBox. Render it at 22 × 22 for
`tray-STATE.png` and at 44 × 44 for `tray-STATE@2x.png`. Tray art uses a single
ink and transparency, with no gradients, background tile, or glow.

| State | Shape | Ink |
| --- | --- | --- |
| connecting | Interrupted orbit with an inactive center | Cyan `#06b6d4` |
| listening | Organic core with two tilted orbits | Cyan `#06b6d4` |
| speaking | Three voice bars with two tilted orbits | Orange `#ff7537` |
| muted | Desaturated orbit with a diagonal slash | Gray `#858585` |
| error | Orbit with an exclamation mark | Coral `#ff6363` |

The existing Rust state mapping embeds the unchanged `tray-STATE.png`
filenames. The `@2x` exports are supplied separately; selecting them at runtime
would require a subsequent change to that mapping. These colored glyphs are
intended for the existing non-template tray configuration.
