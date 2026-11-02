# Asset credits

All files under `assets/do/` were extracted from **Dark Oberon**
(https://dark-oberon.sourceforge.net/, source mirror:
https://github.com/tommybee/dark-oberon) using the scripts in
`tools/doextract.py` and `tools/dosounds.py`.

Dark Oberon is released under the GNU GPL (v2+). These assets are therefore
redistributed under GPL terms, and this project (`rust_oberon`, GPL-3.0-or-later)
is compatible with them.

## Contents

| Directory            | Source archive                | Description                          |
|----------------------|-------------------------------|--------------------------------------|
| `do/gui/`            | `dat/gui.dat`                 | Menu/HUD textures (panels, buttons)  |
| `do/cursors/`        | `dat/cursors.dat`             | Mouse cursor set                     |
| `do/fonts/`          | `dat/fonts.dat`               | Bitmap font texture                  |
| `do/human/`          | `races/human/human.dat`       | Human race sprites (units, buildings, animations) |
| `do/sfx_common/`     | `dat/gui.dat` (sound table)   | Menu/game music & common sounds      |
| `do/sfx_human/`      | `races/human/human.dat` (sound table) | Unit voices, combat & construction sounds |

Each directory contains a `manifest.json` describing texture groups,
animation frame counts (`frames_h` x `frames_v`), frame timing (`anim_ms`)
and sound ids, so the game loader can rebuild sprite sheets and audio maps.

Original Dark Oberon authors: Marek Kopcak, Peter Kovacik, Boris Babicky,
Dusan Salner and others (see `docs/` in the upstream repository).
