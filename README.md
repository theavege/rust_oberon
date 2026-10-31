# Rust Oberon

Rust Oberon is an open-source real-time strategy game written in Rust, inspired by Dark Oberon. The goal is to recover the original game's readable, fast RTS feel while building a maintainable codebase with a deterministic simulation and a modern presentation. Original game assets and data are used with attribution; see [asset credits](assets/CREDITS.md).

## Project plan

### Principles

- Keep simulation rules independent from rendering and UI so game logic stays testable and can support replays or multiplayer later.
- Use a modular monolith: `core` owns shared data, `sim` owns game rules and commands, and `render` owns presentation and camera controls.
- Keep content data-driven, validate imported map and scheme data, and preserve source attribution and license information for every asset set.
- Build complete playable slices before expanding breadth. Prefer simple, inspectable systems over premature framework layers.
- Treat asset legality as a release requirement: the repository records its current understanding in `assets/CREDITS.md`; verify upstream licensing and provenance before publishing redistributed assets.

### Milestones

1. **Playable movement slice (complete)** — load an original map and terrain art; show a controllable camera and a small player squad; select a unit and issue a move order. Done when the game opens into a legible map, WASD/arrows pan the view, left-click selects a unit, and right-click sends it toward the destination.
2. **Simulation foundation (complete)** — represent positions and orders in a fixed-step simulation; derive walkability from the map's Earth layer; route units around blocked terrain with A*; keep simulation state separate from sprites.
3. **First RTS loop (in progress)** — add one resource and harvesting, then one buildable structure and construction placement. Keep the map and gameplay definitions data-driven; add a basic enemy opponent after the loop is playable.
4. **Core combat and visibility** — add attack/health/death, unit roles, fog of war and a useful combat feedback loop.
5. **Persistence and content tools** — add save/load, map/content validation, error reporting and contributor workflows for maps and definitions.
6. **Polish and release** — improve input/UI/audio, performance on larger battles, packaging, documentation, accessibility and release/license notices.

### Shed construction slice (complete)

Press **B** to place a Shed using harvested wood. The grid-snapped preview is green on a valid site and red otherwise; left-click places it, while **Escape** or right-click cancels. Placement checks the full footprint against map bounds, terrain, other buildings and units. The footprint is reserved for navigation immediately, construction progress is visible, and the completed Shed uses the original idle sprite. Invalid sites and insufficient wood cannot spend resources. Footprint, wood cost and construction duration are set by `ShedDefinition`.

### Next step: combat foundations

Build a small, testable combat slice before adding an enemy opponent:

1. Add data-driven damage, range and attack cooldown values for unit kinds.
2. Add an attack command and target handling. Units move into range, attack on cooldown, and stop when the target is dead or invalid.
3. Apply damage and remove units at zero health in the fixed-step simulation, independently of rendering.
4. Add clear visual feedback for health and death.
5. Test attack range, cooldown, damage, death and cancelled or invalid targets.

Done when a player unit can attack an enemy, the enemy takes damage and is removed at zero health, and simulation tests cover those rules. Then add a basic opponent that can attack back.

### Tooling: lint and format

Install Rust, Cargo, Clippy and rustfmt with rustup:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --component clippy --component rustfmt
```

Before committing, check formatting and lints from the repository root:

```sh
cargo fmt --check   # verify formatting (use `cargo fmt` to apply)
cargo clippy --all-targets
```

### First milestone acceptance checklist

- [x] Original map and terrain are visible at a useful scale.
- [x] Camera starts over the playable area and pans with WASD or arrow keys.
- [x] A small squad spawns at the map's player start point.
- [x] Left-click selects a unit and gives visible feedback; clicking empty ground clears selection.
- [x] Right-click issues a move order and the selected unit moves toward it.
- [x] The startup/run instructions and controls are documented here.

## Running

Requires Rust and Cargo. From the repository root:

```sh
cargo run
```

Controls: **WASD / arrow keys** to pan, **left-click** a unit to select (empty ground clears selection), **right-click** open ground to move, and **right-click** a tree with a Peasant selected to harvest wood. Press **B** to place a Shed; **left-click** a valid site, or cancel with **Escape** / right-click.

## Architecture

- `src/core/` — shared types and imported map/scheme data.
- `src/sim/` — fixed-rate game rules, initial setup and player commands.
- `src/render/` — terrain/unit presentation and camera input.
- `assets/` — game assets and maps; see `assets/CREDITS.md` for provenance.

## Contributing

The project is early and APIs are expected to change. For substantial changes, keep simulation and rendering responsibilities separate, document imported assets, and include a concise explanation of how to try the change.
