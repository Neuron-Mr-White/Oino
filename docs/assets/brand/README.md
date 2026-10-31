# oino brand assets

| File | Use |
|---|---|
| `oino.svg` | Master icon: dark rounded tile, light stroke. Works on light and dark backgrounds. |
| `oino-invert.svg` | Master mark: dark stroke on transparent. Light backgrounds only. |
| `oino-square.svg`, `oino-invert-square.svg` | The same, padded to a square canvas for icon export. |
| `png/oino-{16..1024}.png`, `png/oino-invert-{16..1024}.png` | Raster exports of the square versions. |
| `oino.ico` | Windows icon (16–256 px), embedded into `oino.exe` (copy in `crates/oino/assets/`). |
| `social-preview.png` / `.svg` | 1280×640 GitHub social preview. |

Where it's used: the repo README header, every published crate README (crates.io), the
docs.rs logo and favicon (`#![doc(html_logo_url, html_favicon_url)]` in every published crate),
and the Windows `oino.exe` icon. The crates.io and docs.rs references use the raw GitHub URL
on `main`. The UniPi app keeps its own branding.
