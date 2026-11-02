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
3. **First RTS loop (complete)** — add one resource and harvesting, then one buildable structure and construction placement. Keep the map and gameplay definitions data-driven; add a basic enemy opponent after the loop is playable.
4. **Core combat and visibility (complete)** — attack/health/death, distinct unit roles, basic fog of war and combat feedback are implemented.
5. **Persistence and content tools (in progress)** — save/load is implemented; add map/content validation, error reporting and contributor workflows for maps and definitions.
6. **Polish and release** — improve input/UI/audio, performance on larger battles, packaging, documentation, accessibility and release/license notices.

### Shed construction slice (complete)

Press **B** to place a Shed using harvested wood. The grid-snapped preview is green on a valid site and red otherwise; left-click places it, while **Escape** or right-click cancels. Placement checks the full footprint against map bounds, terrain, other buildings and units. The footprint is reserved for navigation immediately, construction progress is visible, and the completed Shed uses the original idle sprite. Invalid sites and insufficient wood cannot spend resources. Footprint, wood cost and construction duration are set by `ShedDefinition`.

### Combat foundations (complete)

The first combat slice is implemented and covered by simulation tests:

1. `CombatDefinition` provides damage, range and cooldown values with per-kind overrides.
2. Attack orders route units into range and resolve attacks on fixed-step cooldowns.
3. Simulation applies damage, rejects invalid targets and removes units at zero health.
4. The view communicates ownership, health loss, selection and death.
5. A basic opponent attacks nearby player units and buildings.

### Fog of war foundation (implemented)

The simulation tracks current vision and persistent exploration for the human player. Each friendly unit reveals a circular radius of eight map cells; vision updates on the fixed simulation tick. Terrain fragments are darkened when explored and blacked out when unexplored, while enemy units, buildings and resource nodes are hidden outside current vision. Rendering and click handling read the visibility state without changing simulation positions.

This first pass uses fragment anchor cells to shade terrain and has a single visibility grid for the current one-human-player game. Add line-of-sight blockers and per-player visibility when the game supports multiple human players.

### Combat feedback (implemented)

Units display health bars and role labels; successful hits show short floating damage numbers. Role colors remain distinct, while simulation-owned combat rules and fog visibility determine the presentation.

### Save/load foundation (implemented)

Press **F5** to write `saves/latest.ron`, or **F9** to load it. The versioned snapshot records map identity, player wood, unit kind/owner/position/health, remaining wood nodes, building footprints and construction progress, plus explored fog. Loading validates version, map, bounds, entity data and building overlap before replacing the live simulation. Save data round-trip, invalid entities and unsupported versions are covered.

Movement and attack orders are not persisted yet; units resume idle after loading. Saves are local to the working directory.

### Content validation command (implemented)

Run `cargo run -- --validate-content` to validate every bundled `.map`, `.sch`, the texture manifest and referenced texture files. The command checks map dimensions, start points, scheme references, fragment/surface bounds and texture paths. Failures include the source path and a line number; pass a custom asset root as the second argument when validating another content pack.

### Runtime load validation (implemented)

`GameState::load` runs the same content validators before parsing and returning a map. Startup inserts the loaded state only after validation succeeds, so invalid content cannot partially replace the live world.

### Campaign load integration (implemented)

Map change requests are routed through `GameState::load` which validates content before loading. The current game state is preserved until the replacement map has passed validation and loaded successfully, ensuring that failed transitions do not corrupt the game state.

### Next step: persist unit orders in save/load

Save/load currently does not persist movement and attack orders; units resume idle after loading. Extend the `Snapshot` format to include active `MovePath` and `AttackOrder` components, then restore them during load so units continue their previous actions. Ensure restored orders are validated against the loaded map bounds and entity existence before applying.

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

Controls: **WASD / arrow keys** to pan, **left-click** a unit to select (empty ground clears selection), **right-click** open ground to move, and **right-click** a tree with a Peasant selected to harvest wood. Press **B** to place a Shed; **left-click** a valid site, or cancel with **Escape** / right-click. **F5** saves and **F9** loads the local save.

## Architecture

- `src/core/` — shared types and imported map/scheme data.
- `src/sim/` — fixed-rate game rules, initial setup and player commands.
- `src/render/` — terrain/unit presentation and camera input.
- `assets/` — game assets and maps; see `assets/CREDITS.md` for provenance.

## Contributing

The project is early and APIs are expected to change. For substantial changes, keep simulation and rendering responsibilities separate, document imported assets, and include a concise explanation of how to try the change.
