//! Simulation layer: deterministic game logic. Runs in `FixedUpdate`.
//! This module must never spawn sprites/meshes — only mutate sim state.

pub mod setup;

use bevy::app::FixedMain;
use bevy::prelude::*;
use bevy::time::{TimeUpdateStrategy, Virtual};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::time::Duration;

use crate::core::{
    map::{NavigationGrid, VisibilityGrid},
    units::{
        AttackCooldown, AttackOrder, Attacking, Building, CombatDefinition, ConstructionSite, Dead,
        HarvestOrder, Health, MovePath, Player, PlayerCommand, PlayerEconomy, ResourceNode,
        Selected, ShedDefinition, ShedPlacement, Unit, UnitKind, WorldPosition,
    },
};
/// Nominal fixed timestep of the simulation, in seconds (20 Hz).
pub const SIM_STEP_SECONDS: f64 = 0.05;

/// Cached nominal fixed timestep (in seconds) as a resource, so fixed-step
/// systems can advance countdown clocks by exactly one step per tick. Bevy
/// only exposes `Time<Fixed>`'s timestep through `State<Time<Fixed>>` (a
/// `Ref`, not a system parameter), and `delta_secs` accumulates the whole
/// catch-up loop — neither is usable for per-tick cooldown math.
#[derive(Resource)]
pub struct SimTimestep(pub f32);

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup::spawn_initial_units)
            .add_event::<PlayerCommand>()
            // `apply_player_commands` also runs on `Update` in production;
            // scheduling it here as well is a no-op there because its event
            // reader consumes every `PlayerCommand` the first time it runs.
            // In tests, where commands are issued from `Update`, running it
            // before the fixed combat step lets a freshly issued order act
            // within the same tick — matching how deferred writes flush at
            // the end of an update in production.
            .add_systems(Update, update_shed_placement)
            // The simulation runs on `FixedMain`, driven at a fixed frequency
            // by the time plugin's catch-up loop. Tests drive the same
            // schedule deterministically with a manual virtual clock.
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                SIM_STEP_SECONDS,
            )))
            .insert_resource(SimTimestep(SIM_STEP_SECONDS as f32))
            // `apply_player_commands` also runs on `Update` in production;
            // scheduling it here as well is a no-op there because its event
            // reader consumes every `PlayerCommand` the first time it runs.
            // In tests, where commands are issued from `Update`, running it
            // before the fixed combat step lets a freshly issued order act
            // within the same tick — matching how deferred writes flush at
            // the end of an update in production.
            .add_systems(
                FixedMain,
                (
                    apply_player_commands,
                    opponent_ai_system.after(apply_player_commands),
                    movement_system.after(opponent_ai_system),
                    harvest_system.after(movement_system),
                    construction_system.after(harvest_system),
                    attack_system.after(construction_system),
                    death_system.after(attack_system),
                    update_visibility.after(death_system),
                ),
            );
    }
}

/// Recompute current player vision after movement/combat; explored cells are
/// deliberately retained by `VisibilityGrid`.
fn update_visibility(
    navigation: Res<NavigationGrid>,
    mut visibility: ResMut<VisibilityGrid>,
    scouts: Query<(&Player, &WorldPosition), With<Unit>>,
) {
    if visibility.width != navigation.width || visibility.height != navigation.height {
        *visibility = VisibilityGrid::from_navigation(&navigation);
    }
    visibility.clear_visible();
    const SCOUT_RADIUS: i32 = 8;
    for (owner, position) in &scouts {
        if owner.0 == 0 {
            visibility.reveal(navigation.world_to_cell(position.0), SCOUT_RADIUS);
        }
    }
}

fn apply_player_commands(
    mut messages: EventReader<PlayerCommand>,
    mut commands: Commands,
    mut navigation: ResMut<NavigationGrid>,
    mut economy: ResMut<PlayerEconomy>,
    shed: Res<ShedDefinition>,
    units: Query<(Entity, &Unit, Option<&Selected>, &WorldPosition), With<Player>>,
    nodes: Query<&WorldPosition, (With<ResourceNode>, Without<Unit>)>,
    enemy_targets: Query<
        (&Health, &Player, &WorldPosition, Option<&Building>),
        Or<(With<Unit>, With<Building>)>,
    >,
    other_units: Query<&WorldPosition, With<Unit>>,
    combat: Res<CombatDefinition>,
) {
    for message in messages.read() {
        match *message {
            PlayerCommand::Select(target) => {
                for (entity, _, selected, _) in &units {
                    if selected.is_some() {
                        commands.entity(entity).remove::<Selected>();
                    }
                }
                if let Some(entity) = target {
                    commands.entity(entity).insert(Selected);
                }
            }
            PlayerCommand::MoveSelected(destination) => {
                let requested = navigation.world_to_cell(destination);
                let goal = navigation.nearest_walkable(requested);
                for (entity, _, selected, position) in &units {
                    if selected.is_some() {
                        let start = navigation.world_to_cell(position.0);
                        let path = goal
                            .and_then(|goal| navigation.find_path(start, goal))
                            .unwrap_or_default();
                        commands
                            .entity(entity)
                            .remove::<HarvestOrder>()
                            .remove::<AttackOrder>()
                            .remove::<Attacking>()
                            .insert(MovePath(path.into()));
                    }
                }
            }
            PlayerCommand::Harvest(target) => {
                let Ok(node_position) = nodes.get(target) else {
                    continue;
                };
                let goal = navigation.nearest_walkable(navigation.world_to_cell(node_position.0));
                for (entity, unit, selected, position) in &units {
                    if selected.is_none() || unit.kind != UnitKind::Peasant {
                        continue;
                    }
                    let start = navigation.world_to_cell(position.0);
                    let path = goal
                        .and_then(|goal| navigation.find_path(start, goal))
                        .unwrap_or_default();
                    commands.entity(entity).insert((
                        MovePath(path.into()),
                        HarvestOrder {
                            target,
                            elapsed: 0.0,
                        },
                    ));
                }
            }
            PlayerCommand::PlaceShed(origin) => {
                let occupied_cells: Vec<IVec2> = units
                    .iter()
                    .map(|(_, _, _, position)| navigation.world_to_cell(position.0))
                    .collect();
                if economy.wood < shed.wood_cost
                    || !can_place_shed(&navigation, origin, shed.footprint, &occupied_cells)
                    || !navigation.reserve_footprint(origin, shed.footprint)
                {
                    continue;
                }

                economy.wood -= shed.wood_cost;
                commands.spawn((
                    Building {
                        owner: 0,
                        footprint: shed.footprint,
                    },
                    ConstructionSite {
                        elapsed: 0.0,
                        duration: shed.construction_seconds.max(0.1),
                    },
                    WorldPosition(footprint_center(origin, shed.footprint)),
                ));
            }
            PlayerCommand::Attack(target) => {
                // Validate the target: it must be a living enemy unit or an
                // enemy building (no friendly fire, no attacking scenery).
                let Ok((target_health, target_player, target_position, target_building)) =
                    enemy_targets.get(target)
                else {
                    continue;
                };
                if target_player.0 == 0 || target_health.current <= 0.0 {
                    continue;
                }
                for (entity, unit, selected, position) in &units {
                    if selected.is_none() || !combat.can_attack(unit.kind) {
                        continue;
                    }
                    let range = combat.range_for(unit.kind);
                    // The target's strikeable cells: its footprint when it is
                    // a building, otherwise the single cell it stands on. All
                    // reach checks and approach searches use world distance to
                    // this rectangle, so archers snipe from afar and melee
                    // units walk right up next to their prey.
                    let cells = target_cells_for(
                        |point| navigation.world_to_cell(point),
                        target_building,
                        target_position.0,
                    );
                    let mut entity_commands = commands.entity(entity);
                    if rect_distance(position.0, &cells) > range {
                        // Out of reach: queue the approach route and attach the
                        // order only once the path exists (see below).
                        let ctx = make_approach_context(
                            &navigation,
                            position.0,
                            cells,
                            other_units.iter().filter(|other| other.0 != position.0),
                            range,
                            |grid, point| grid.world_to_cell(point),
                        );
                        let path = approach_path(&ctx, APPROACH_EXPANSION_BOUND);
                        if path.is_empty() {
                            // No reachable attack position: do not leave a
                            // permanent order against a target this unit can
                            // never strike.
                            continue;
                        }
                        entity_commands.insert(MovePath(path.into()));
                    } else {
                        // Already in range: drop any leftover movement route
                        // so the unit stands its ground and strikes.
                        entity_commands.remove::<MovePath>();
                    }
                    entity_commands.insert((
                        AttackOrder { target },
                        AttackCooldown::default(),
                        Attacking,
                    ));
                }
            }
        }
    }
}

/// Cell step costs for the approach search: orthogonal 10, diagonal 14 —
/// identical to `NavigationGrid::find_path` so route lengths stay comparable.
const CELL_STEP_COST: u32 = 10;
const CELL_DIAG_STEP_COST: u32 = 14;

/// Octile-style cell distance (same metric as `NavigationGrid::find_path`'s
/// heuristic), used both as the A* heuristic and as an admissible lower bound
/// on world distance between two cell centres.
fn heuristic_cells(a: IVec2, b: IVec2) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    14 * dx.min(dy) + 10 * (dx.max(dy) - dx.min(dy))
}

/// Distance from `position` to the closest point of the axis-aligned
/// rectangle spanned by `cells`, in isometric world units. The rectangle is
/// padded by half a tile (8 px horizontally, 4 px vertically) so cells whose
/// centres sit just outside the footprint still count as covering it. Used to
/// decide whether a unit standing at `position` can strike a multi-cell
/// target (buildings) or a single-cell unit with `range`.
fn rect_distance(position: Vec2, cells: &[IVec2]) -> f32 {
    if cells.is_empty() {
        return f32::INFINITY;
    }
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for cell in cells {
        let center = NavigationGrid::cell_center(*cell);
        min_x = min_x.min(center.x);
        max_x = max_x.max(center.x);
        min_y = min_y.min(center.y);
        max_y = max_y.max(center.y);
    }
    min_x -= 8.0;
    max_x += 8.0;
    min_y -= 4.0;
    max_y += 4.0;
    let dx = (min_x - position.x).max(0.0).max(position.x - max_x);
    let dy = (min_y - position.y).max(0.0).max(position.y - max_y);
    (dx * dx + dy * dy).sqrt()
}

/// Lower bound on the world distance between the centres of two cells under
/// the isometric projection (a horizontal neighbour step moves 16 px, a
/// vertical one 8 px, so the cheapest per-cell displacement is 8 px).
fn cell_world_lower_bound(from: IVec2, to: IVec2) -> f32 {
    let dx = (from.x - to.x).unsigned_abs();
    let dy = (from.y - to.y).unsigned_abs();
    // Cheapest case: pure vertical movement at 8 px per cell.
    8.0 * (dx + dy) as f32
}

/// Static inputs for one attack-approach search: the grid, the attacker's
/// start cell, the target's footprint cells, the cells that must not be used
/// as the final stopping cell, and the combat range. All query data is
/// collected into plain vectors before the search starts so the loop borrows
/// nothing from the Bevy world.
pub struct ApproachContext<'a> {
    pub navigation: &'a NavigationGrid,
    pub start: IVec2,
    /// Cells occupied by the target itself (its footprint). Routes may cross
    /// them but never end on them.
    pub target_cells: Vec<IVec2>,
    /// Cells off-limits as the *stopping* cell: reserved building footprints
    /// plus other living units' current cells (units do not physically stack).
    pub blocked_stop: Vec<IVec2>,
    pub range: f32,
}

impl ApproachContext<'_> {
    fn index(&self, cell: IVec2) -> usize {
        self.navigation.cell_index(cell)
    }

    fn in_bounds(&self, cell: IVec2) -> bool {
        cell.x >= 0
            && cell.y >= 0
            && cell.x < self.navigation.width as i32
            && cell.y < self.navigation.height as i32
    }

    /// Passable for *traversal*: terrain only. Reserved footprints are legal
    /// to walk across en route (they belong to dynamic objects such as
    /// construction sites); routes simply refuse to *end* on them.
    fn traversable(&self, cell: IVec2) -> bool {
        self.in_bounds(cell) && self.navigation.is_terrain_walkable(cell)
    }

    /// True when a unit standing on `cell` can hit the target with `range`.
    fn covers_target(&self, cell: IVec2) -> bool {
        rect_distance(NavigationGrid::cell_center(cell), &self.target_cells) <= self.range
    }

    /// Admissible A* heuristic: minimum possible world distance from `cell`
    /// to any target cell, minus the weapon range. Zero exactly where the
    /// unit can already strike, and never overestimates the remaining cost
    /// (measured in world pixels), so the first strike-capable cell the search
    /// pops is reached by a shortest-cost route among the explored region.
    fn heuristic(&self, cell: IVec2) -> f32 {
        let mut best = f32::INFINITY;
        for &target in &self.target_cells {
            let bound = cell_world_lower_bound(cell, target) - self.range;
            if bound < best {
                best = bound;
            }
        }
        best.max(0.0)
    }
}

/// Upper bound on A* expansions per attack-approach search. On the shipped
/// map this comfortably covers cross-map routes while keeping a pathological
/// (e.g. walled-off) target from stalling a fixed-step tick.
const APPROACH_EXPANSION_BOUND: usize = 20_000;

/// Collect the static inputs for one attack-approach search out of the world:
/// the target's own cells (footprint for buildings, current cell for units),
/// the cells other living units currently occupy (attackers never stop on top
/// of a third party), and the attacker's start cell. Everything is copied into
/// plain data so `approach_path` borrows nothing from the Bevy world.
///
/// `cell_of` maps a world position to its navigation cell; tests override it
/// because their virtual units do not sit exactly on cell centres.
fn make_approach_context<'a>(
    navigation: &'a NavigationGrid,
    attacker_position: Vec2,
    target_cells: Vec<IVec2>,
    other_units: impl Iterator<Item = &'a WorldPosition>,
    range: f32,
    cell_of: impl Fn(&'a NavigationGrid, Vec2) -> IVec2,
) -> ApproachContext<'a> {
    let blocked_stop: Vec<IVec2> = other_units
        .map(|position| cell_of(navigation, position.0))
        .collect();
    ApproachContext {
        navigation,
        start: cell_of(navigation, attacker_position),
        target_cells,
        blocked_stop,
        range,
    }
}

/// Cells belonging to an attack target entity: the full footprint for
/// buildings (under construction or complete), otherwise the single cell the
/// unit currently stands on. `cell_of` converts a world position to its
/// navigation cell (production uses the grid's inverse projection; tests use
/// rounding because their virtual units do not sit exactly on cell centres).
fn target_cells_for(
    cell_of: impl Fn(Vec2) -> IVec2,
    building: Option<&Building>,
    target_position: Vec2,
) -> Vec<IVec2> {
    if let Some(building) = building {
        let origin = cell_of(target_position);
        let footprint = building.footprint;
        (0..footprint.y)
            .flat_map(|dy| (0..footprint.x).map(move |dx| origin + IVec2::new(dx, dy)))
            .collect()
    } else {
        vec![cell_of(target_position)]
    }
}

/// Path an attacking unit walks toward a target, aiming to reach a cell from
/// which it can strike — i.e. whose world position lies within `range` of the
/// target's footprint rectangle — instead of walking onto the target itself.
///
/// This is a bounded A* over the navigation grid whose goal set is "any cell
/// that covers the target" rather than a single destination, so the route
/// naturally bends around walls, water and building footprints and stops at
/// the first cell in range. Traversal ignores reserved footprints (routes may
/// cross them), while the chosen stopping cell avoids the target's own cells,
/// reserved cells and other units' positions. Diagonal corner cutting is
/// forbidden, matching `NavigationGrid::find_path`.
///
/// The search expands at most `bound` cells. When no reachable strike position
/// exists inside that budget — e.g. a target walled off behind impassable
/// terrain — the function returns an empty vector and callers reject the
/// order, leaving the unit holding no permanent unreachable attack.
fn approach_path(ctx: &ApproachContext, bound: usize) -> Vec<IVec2> {
    if !ctx.traversable(ctx.start) {
        return Vec::new();
    }
    // Priority queue keyed by (f-score in milli-pixels, insertion-order
    // tiebreak, cell index); BinaryHeap is a max-heap so entries are reversed.
    let mut heap: BinaryHeap<std::cmp::Reverse<(i64, u64, usize)>> = BinaryHeap::new();
    let mut g_score: HashMap<usize, u32> = HashMap::new();
    let mut came_from: HashMap<usize, usize> = HashMap::new();
    let start_idx = ctx.index(ctx.start);
    g_score.insert(start_idx, 0);
    let mut counter = 0u64;
    heap.push(std::cmp::Reverse((
        (ctx.heuristic(ctx.start) * 1000.0) as i64,
        counter,
        start_idx,
    )));
    counter += 1;

    const NEIGHBORS: [(i32, i32); 8] = [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (-1, 1),
        (1, -1),
        (1, 1),
    ];

    let mut visited: HashSet<usize> = HashSet::new();
    let mut expanded = 0usize;
    while let Some(std::cmp::Reverse((_, _, idx))) = heap.pop() {
        if !visited.insert(idx) {
            continue;
        }
        expanded += 1;
        if expanded > bound {
            break;
        }
        let cell = IVec2::new(
            (idx as u32 % ctx.navigation.width) as i32,
            (idx as u32 / ctx.navigation.width) as i32,
        );
        // Goal test: a cell the attacker can stand on and strike from. The
        // start cell is skipped so an out-of-range attacker always produces a
        // non-empty route (callers treat an empty path as "no approach").
        if cell != ctx.start
            && ctx.covers_target(cell)
            && !ctx.target_cells.contains(&cell)
            && !ctx.blocked_stop.contains(&cell)
        {
            return reconstruct_path(&came_from, start_idx, idx, ctx.navigation);
        }
        let current_cost = g_score[&idx];
        for (dx, dy) in NEIGHBORS {
            let next = cell + IVec2::new(dx, dy);
            if !ctx.traversable(next) {
                continue;
            }
            if dx != 0 && dy != 0 {
                // No corner cutting through blocked terrain.
                if !ctx.traversable(IVec2::new(cell.x + dx, cell.y))
                    || !ctx.traversable(IVec2::new(cell.x, cell.y + dy))
                {
                    continue;
                }
            }
            let step_cost = if dx == 0 || dy == 0 {
                CELL_STEP_COST
            } else {
                CELL_DIAG_STEP_COST
            };
            let new_cost = current_cost + step_cost;
            let next_idx = ctx.index(next);
            if g_score.get(&next_idx).is_some_and(|&old| old <= new_cost) {
                continue;
            }
            g_score.insert(next_idx, new_cost);
            came_from.insert(next_idx, idx);
            let f = (new_cost as f32 * 8.0 + ctx.heuristic(next)) * 1000.0;
            heap.push(std::cmp::Reverse((
                (f.round() as i64).max(0),
                counter,
                next_idx,
            )));
            counter += 1;
        }
    }
    Vec::new()
}

/// Walk the `came_from` parent links backwards from `end` to `start` and
/// return the forward list of cells (excluding `start`, matching
/// `NavigationGrid::find_path`'s contract).
fn reconstruct_path(
    came_from: &HashMap<usize, usize>,
    start_idx: usize,
    end_idx: usize,
    navigation: &NavigationGrid,
) -> Vec<IVec2> {
    let width = navigation.width;
    let mut path = Vec::new();
    let mut cursor = end_idx;
    while cursor != start_idx {
        path.push(IVec2::new(
            (cursor as u32 % width) as i32,
            (cursor as u32 / width) as i32,
        ));
        match came_from.get(&cursor) {
            Some(&parent) => cursor = parent,
            None => return Vec::new(),
        }
    }
    path.reverse();
    path
}

fn can_place_shed(
    navigation: &NavigationGrid,
    origin: IVec2,
    footprint: IVec2,
    occupied_cells: &[IVec2],
) -> bool {
    navigation.can_reserve_footprint(origin, footprint)
        && !occupied_cells
            .iter()
            .any(|&cell| footprint_contains(origin, footprint, cell))
}

fn footprint_contains(origin: IVec2, footprint: IVec2, cell: IVec2) -> bool {
    cell.x >= origin.x
        && cell.y >= origin.y
        && cell.x < origin.x + footprint.x
        && cell.y < origin.y + footprint.y
}

fn footprint_center(origin: IVec2, footprint: IVec2) -> Vec2 {
    let offset = Vec2::new(
        (footprint.x - 1) as f32 / 2.0,
        (footprint.y - 1) as f32 / 2.0,
    );
    NavigationGrid::cell_center(origin)
        + Vec2::new(16.0 * (offset.x - offset.y), -8.0 * (offset.x + offset.y))
}

fn update_shed_placement(
    navigation: Res<NavigationGrid>,
    economy: Res<PlayerEconomy>,
    shed: Res<ShedDefinition>,
    units: Query<&WorldPosition, (With<Unit>, With<Player>)>,
    mut placement: ResMut<ShedPlacement>,
) {
    if !placement.active {
        placement.valid = false;
        return;
    }
    let occupied_cells: Vec<IVec2> = units
        .iter()
        .map(|position| navigation.world_to_cell(position.0))
        .collect();
    placement.valid = economy.wood >= shed.wood_cost
        && can_place_shed(
            &navigation,
            placement.origin,
            shed.footprint,
            &occupied_cells,
        );
}

fn harvest_system(
    time: Res<Time<Fixed>>,
    mut economy: ResMut<PlayerEconomy>,
    mut commands: Commands,
    mut workers: Query<(Entity, &WorldPosition, &mut HarvestOrder), (With<Unit>, With<Player>)>,
    mut nodes: Query<(&WorldPosition, &mut ResourceNode), Without<Unit>>,
) {
    for (worker, position, mut order) in &mut workers {
        let Ok((node_position, mut node)) = nodes.get_mut(order.target) else {
            commands.entity(worker).remove::<HarvestOrder>();
            continue;
        };
        if position.0.distance(node_position.0) > 42.0 {
            continue;
        }

        order.elapsed += time.delta_secs();
        while order.elapsed >= 1.0 && node.wood > 0 {
            order.elapsed -= 1.0;
            node.wood -= 1;
            economy.wood += 1;
        }
        if node.wood == 0 {
            commands.entity(order.target).despawn();
            commands.entity(worker).remove::<HarvestOrder>();
        }
    }
}

fn construction_system(
    time: Res<Time<Fixed>>,
    mut commands: Commands,
    mut sites: Query<(Entity, &mut ConstructionSite)>,
) {
    for (entity, mut site) in &mut sites {
        site.elapsed += time.delta_secs();
        if site.elapsed >= site.duration {
            commands.entity(entity).remove::<ConstructionSite>();
        }
    }
}

/// Move units toward their current destination in isometric world space.
fn movement_system(
    time: Res<Time<Fixed>>,
    navigation: Res<NavigationGrid>,
    mut q: Query<(&mut WorldPosition, &Unit, &mut MovePath), With<Unit>>,
) {
    let dt = time.delta_secs();
    for (mut position, unit, mut path) in q.iter_mut() {
        let Some(&next_cell) = path.0.front() else {
            continue;
        };
        let pos = position.0;
        let current_cell = navigation.world_to_cell(pos);
        let diagonal = current_cell.x != next_cell.x && current_cell.y != next_cell.y;
        if !navigation.is_walkable(next_cell)
            || (diagonal
                && (!navigation.is_walkable(IVec2::new(next_cell.x, current_cell.y))
                    || !navigation.is_walkable(IVec2::new(current_cell.x, next_cell.y))))
        {
            path.0.clear();
            continue;
        }
        let target = NavigationGrid::cell_center(next_cell);
        let delta = target - pos;
        let dist = delta.length();
        let step = unit.kind.speed() * 32.0 * dt;
        if dist <= step {
            position.0 = target;
            path.0.pop_front();
            continue;
        }
        if dist < 0.01 {
            continue;
        }
        let next = pos + delta.normalize() * step;
        position.0 = next;
    }
}

/// Fixed-step combat resolution: units with an `AttackOrder` move toward their
/// target (handled by `movement_system`) and strike whenever the target is in
/// range and the cooldown has elapsed. Invalid or dead targets are dropped.
/// The system reads one consistent snapshot per tick, decides every strike up
/// front, then applies damage so a second hit inside the same tick sees the
/// reduced health. Structural changes go through deferred `Commands`.
///
/// The two mutable queries are deliberately disjoint so Bevy's scheduler can
/// prove they never touch the same entity: fighters are exactly the units
/// holding an `AttackOrder`, and pure victims are everything else
/// (`Without<AttackOrder>`). A freshly ordered attacker appears in the
/// fighter query on the very next fixed step, because orders flush into the
/// world at the end of the current tick.
#[allow(clippy::type_complexity)]
fn attack_system(
    timestep: Res<SimTimestep>,
    combat: Res<CombatDefinition>,
    mut commands: Commands,
    // Fighters — units holding an `AttackOrder` — together with their
    // cooldown clocks and health. Cooldown and attacker-health writes land
    // directly here; the decision pass snapshots from this query before
    // mutating through it, so a fighter struck by several enemies
    // accumulates one merged in-place write per target.
    mut fighters: Query<
        (
            Entity,
            &Unit,
            &WorldPosition,
            &Player,
            &AttackOrder,
            Option<&mut AttackCooldown>,
            &mut Health,
        ),
        (With<Unit>, With<AttackOrder>),
    >,
    // Pure victims — every unit not holding an attack order this tick, plus
    // buildings (which never fight back). The fighter query filters
    // `With<AttackOrder>` and this one filters `Without<AttackOrder>`, so no
    // entity can appear in both queries and the scheduler can prove their
    // `Health` access disjoint even though both mutate it: damage lands in
    // place and a second strike inside the same tick sees the reduced health.
    #[allow(clippy::type_complexity)] mut victims: Query<
        (Entity, &Player, &WorldPosition, &mut Health),
        (Without<AttackOrder>, Or<(With<Unit>, With<Building>)>),
    >,
) {
    // The cooldown is a countdown clock, so it must advance by exactly one
    // fixed step per tick. `Time<Fixed>::delta_secs` reports the accumulated
    // real-time delta of the whole catch-up loop (and drifts with the wall
    // clock under the virtual clock in tests), which would make strikes
    // non-deterministic; the nominal timestep is the production cadence.
    // `Time<Fixed>` only exposes the timestep through `State<Time<Fixed>>`
    // (a `Ref`, not a system param), so read it once and cache it as a
    // resource; `SimPlugin::build` inserts the same constant up front.
    let dt = timestep.0;
    // Half-step timing offset for cooldown clocks. The fixed-step driver runs
    // `Time::tick` *before* the first `FixedMain` system of each tick, so a
    // clock written by a strike in tick N is already decayed once (by the
    // advance at the top of tick N+1) before that tick's decision pass reads
    // it. Decay-before-check alone would therefore gate strikes to every
    // ceil(cooldown / dt) + 1 ticks — one full step slower than the nominal
    // cadence. Storing `cooldown - dt/2` centres the clock between two
    // advances so exactly `cooldown / dt` ticks elapse between strikes.
    const COOLDOWN_PHASE: f32 = SIM_STEP_SECONDS as f32 * 0.5;
    // Snapshot every unit up front: a lethal strike inside this same step
    // must not stop another attacker from also landing its blow, and the
    // damage pass below never re-reads state while a query is iterating.
    // Victims (units without an attack order) come from the read-only
    // side of the split; fighters snapshot their own rows further below.
    #[allow(clippy::type_complexity)]
    let mut snapshot: Vec<(Entity, u8, Vec2, f32)> = Vec::new();
    for (entity, player, position, health) in victims.iter() {
        snapshot.push((entity, player.0, position.0, health.current));
    }
    // Map from entity to its snapshotted row, for O(1) target lookups. The
    // health column freezes the HP seen at the top of the tick; later
    // in-place writes are tracked through `victim_health`, so staleness
    // checks use this value only to decide whether an order started the
    // tick against a living enemy.
    #[allow(clippy::type_complexity)]
    let rows: HashMap<Entity, (u8, Vec2, f32)> = snapshot
        .iter()
        .map(|&(entity, player, position, health)| (entity, (player, position, health)))
        .collect();
    std::mem::drop(snapshot);

    // Snapshot the fighters (units holding an `AttackOrder`) together with
    // their live health and cooldown clocks before any mutable pass runs; the
    // decision loop below works purely on this data, and every write goes
    // back through the fighter query afterwards. Fighters are also valid
    // attack targets, so keep lookup tables for their rows as well — the
    // damage pass uses `attacking_rows` to route hits to the in-place
    // fighter query instead of the disjoint victim query.
    #[allow(clippy::type_complexity)]
    let fighter_snapshot: Vec<(Entity, UnitKind, Vec2, u8, Entity, f32, Option<f32>)> = fighters
        .iter()
        .map(|(entity, unit, position, owner, order, cooldown, health)| {
            (
                entity,
                unit.kind,
                position.0,
                owner.0,
                order.target,
                health.current,
                cooldown.map(|cooldown| cooldown.0),
            )
        })
        .collect();
    let attacking_rows: HashSet<Entity> = fighter_snapshot.iter().map(|row| row.0).collect();
    #[allow(clippy::type_complexity)]
    let fighter_rows: HashMap<Entity, (u8, Vec2, f32)> = fighter_snapshot
        .iter()
        .map(|&(entity, _, position, owner, _, health, _)| (entity, (owner, position, health)))
        .collect();

    // Decide every strike for this tick first, then apply them in one pass.
    let mut pending_damage: Vec<(Entity, f32)> = Vec::new();
    let mut stale_orders: Vec<Entity> = Vec::new();
    // (entity, value, struck_this_tick). A unit that just attacked is set to
    // its full cooldown; one that merely waited has the clock decayed by one
    // step. The two cases must never be combined for the same entity.
    let mut strike_cooldowns: Vec<(Entity, f32, bool)> = Vec::new();
    // Fighters that take damage this tick, merged per target so several
    // blows landing within one tick accumulate into a single in-place write.
    let mut fighter_damage: HashMap<Entity, f32> = HashMap::new();
    // Cooldown bookkeeping for attackers that are still out of range and do
    // not carry the component yet — inserting it here would overlap the
    // mutable fighter query above, so these few writes stay deferred.
    let mut ensure_cooldown: Vec<Entity> = Vec::new();

    for &(entity, kind, position, owner, target, health, cooldown) in &fighter_snapshot {
        // Drop stale orders: the target must still exist, be alive, and not
        // belong to the attacker's own player (no friendly fire). Targets may
        // be pure victims or fellow fighters, so look in both row sources.
        let target_ok = target_is_valid(&rows, &fighter_rows, target, owner);
        if !target_ok {
            stale_orders.push(entity);
            continue;
        }

        let distance = target_distance(&rows, &fighter_rows, target, position);
        // The cooldown is a countdown clock: a missing component means the
        // unit has never struck and is ready right now. Decay happens once,
        // up front, regardless of range — an attacker walking toward its
        // target must keep ticking every tick, or it would arrive out of
        // phase with its own clock and wait a full extra cycle (or strike
        // instantly) on arrival.
        let remaining = cooldown.map(|clock| clock - dt);
        let ready = remaining.unwrap_or(0.0) <= 0.0;
        if distance > combat.range_for(kind) {
            if let Some(decayed) = remaining {
                strike_cooldowns.push((entity, decayed, false));
            } else {
                ensure_cooldown.push(entity);
            }
            continue;
        }
        if !ready {
            // In range but still cooling down: record this tick's decay.
            strike_cooldowns.push((entity, remaining.unwrap(), false));
            continue;
        }

        let amount = combat.damage_for(kind);
        if attacking_rows.contains(&target) {
            // The victim is a fighter: merge its blows and apply them
            // in place through the fighter query below.
            let taken = fighter_damage.entry(target).or_insert(0.0);
            *taken = (*taken + amount).min(health.max(0.0));
        } else {
            pending_damage.push((target, amount));
        }
        // See `COOLDOWN_PHASE`: the stored clock is offset by half a step so
        // the next strike lands exactly one cooldown later, not one tick
        // later. The ready check (`remaining <= 0`) is unaffected because the
        // offset only shifts *when* the clock crosses zero relative to the
        // driver's per-tick advance.
        strike_cooldowns.push((entity, combat.cooldown_for(kind) - COOLDOWN_PHASE, true));
    }

    // Units that hold an attack order always own their cooldown component
    // (every order assignment inserts one), so cooldown writes land directly
    // on the fighter query. The bool flag distinguishes two cases: a unit
    // that just attacked gets its full cooldown written even when the decay
    // pass already recorded this tick's clock update, while a unit that
    // merely waited has the decayed value written instead. Anything
    // unexpected falls back to a deferred insert. Fighters whose order was
    // removed this tick (stale targets) take their cooldown through the same
    // deferred path instead, because applying structural edits mid-system is
    // unsafe.
    let demoted: HashSet<Entity> = stale_orders.iter().copied().collect();
    for entity in &stale_orders {
        commands.entity(*entity).remove::<AttackOrder>();
    }
    // Sync the `Attacking` marker with the orders decided above: every
    // fighter keeps it while its target stays valid, and units whose order
    // was dropped this tick lose it immediately. Removals apply before
    // insertions during command flushing, so an attacker that loses its
    // order this tick is never left marked — which keeps the victim query's
    // disjointness provable to the scheduler on the next tick.
    for &(entity, _, _, owner, target, _, _) in &fighter_snapshot {
        if !demoted.contains(&entity) && target_is_valid(&rows, &fighter_rows, target, owner) {
            commands.entity(entity).insert(Attacking);
        }
    }
    for entity in &demoted {
        commands.entity(*entity).remove::<Attacking>();
    }
    for entity in &ensure_cooldown {
        commands.entity(*entity).insert(AttackCooldown(0.0));
    }
    for (entity, value, struck) in strike_cooldowns {
        if demoted.contains(&entity) {
            commands.entity(entity).insert(AttackCooldown(value));
        } else if let Ok((_, _, _, _, _, Some(mut cooldown), _)) = fighters.get_mut(entity) {
            // A unit that just attacked is set to its full cooldown no matter
            // what the decay pass did this tick; one that merely waited has
            // the decayed clock value.
            cooldown.0 = if struck { value.max(0.0) } else { value };
        } else {
            commands.entity(entity).insert(AttackCooldown(value));
        }
    }

    // Apply the accumulated damage in strike order and mark anyone who
    // reaches zero as dead; `death_system` despawns them. Pure victims are
    // edited in place through the disjoint victim query, so a second strike
    // inside this same tick sees the reduced health. Fighters are also
    // damaged in place — through the fighter query, one merged write per
    // target — because their `Health` cannot be touched through the victim
    // query without aliasing.
    for (victim, amount) in pending_damage {
        if let Ok((_, _, _, mut health)) = victims.get_mut(victim) {
            if health.take_damage(amount) {
                commands.entity(victim).insert(Dead);
            }
        }
        // A victim that vanished between the snapshot and now simply takes
        // no blow; `take_damage` clamps at zero so two strikes can never
        // overkill past the unit's remaining health.
    }
    for (victim, total) in fighter_damage {
        if let Ok((_, _, _, _, _, _, mut health)) = fighters.get_mut(victim) {
            if health.take_damage(total) {
                commands.entity(victim).insert(Dead);
            }
        }
    }
}

/// True when `target` names a living enemy unit in either row table: the
/// pure-victim snapshot or the fighter snapshot (attackers are valid targets
/// too). Used by `attack_system` both for staleness checks and for keeping
/// the `Attacking` marker in sync with live orders.
#[allow(clippy::type_complexity)]
fn target_is_valid(
    victim_rows: &HashMap<Entity, (u8, Vec2, f32)>,
    fighter_rows: &HashMap<Entity, (u8, Vec2, f32)>,
    target: Entity,
    owner: u8,
) -> bool {
    let row = victim_rows
        .get(&target)
        .or_else(|| fighter_rows.get(&target));
    row.is_some_and(|(player, _, health)| *player != owner && *health > 0.0)
}

/// Distance from `position` to the snapshotted location of `target`, looked
/// up in the victim rows first and then the fighter rows. Returns infinity
/// when the target is missing from both tables.
#[allow(clippy::type_complexity)]
fn target_distance(
    victim_rows: &HashMap<Entity, (u8, Vec2, f32)>,
    fighter_rows: &HashMap<Entity, (u8, Vec2, f32)>,
    target: Entity,
    position: Vec2,
) -> f32 {
    victim_rows
        .get(&target)
        .or_else(|| fighter_rows.get(&target))
        .map_or(f32::INFINITY, |(_, target_position, _)| {
            position.distance(*target_position)
        })
}

/// Remove units that reached zero health inside the fixed-step simulation,
/// independently of rendering.
fn death_system(mut commands: Commands, dead: Query<Entity, With<Dead>>) {
    for entity in &dead {
        commands.entity(entity).despawn();
    }
}

/// Basic opponent: enemy (player 1) units pick the closest living unit of
/// another player as their target and move into range when needed. The regular
/// `attack_system` then resolves these orders with the same cooldown and damage
/// rules as player attacks, keeping the AI purely simulation-side. Player-0
/// units are never auto-ordered; they only fight on explicit commands.
fn opponent_ai_system(
    combat: Res<CombatDefinition>,
    navigation: Res<NavigationGrid>,
    mut commands: Commands,
    opponents: Query<
        (Entity, &Unit, &WorldPosition, Option<&AttackOrder>),
        (With<Player>, With<Health>),
    >,
    victims: Query<(Entity, &Player, &WorldPosition, &Health), With<Unit>>,
    players: Query<&Player>,
) {
    for (entity, unit, position, order) in &opponents {
        let Ok(owner) = players.get(entity) else {
            continue;
        };
        if owner.0 != 1 || !combat.can_attack(unit.kind) {
            continue;
        }
        // Already ordered against a living enemy: keep that target choice
        // (no ping-ponging between candidates), but do not return early —
        // the unit must keep re-routing toward the target until it is in
        // range. An order assigned while out of reach with no path yet is
        // otherwise inert: nothing else ever queues an approach for AI units,
        // so they would stand still forever next to a distant victim. The
        // routing block below runs every tick for ordered attackers too;
        // once in range it simply clears any leftover movement route.
        let already_ordered = order.is_some_and(|order| {
            victims
                .get(order.target)
                .is_ok_and(|(_, player, _, health)| player.0 != owner.0 && health.current > 0.0)
        });
        // Keep an existing valid order's target choice; only re-acquire when
        // the current order is missing or points at a friendly/dead/vanished
        // unit. Deterministic tie-break (see `target_preference_key`): nearest
        // first, then earliest-spawned entity, so identical worlds always pick
        // the same target regardless of query iteration order.
        let held_target = order.map(|order| order.target).filter(|target| {
            victims
                .get(*target)
                .is_ok_and(|(_, player, _, health)| player.0 != owner.0 && health.current > 0.0)
        });
        let Some((target, target_position)) = held_target
            .and_then(|target| {
                victims
                    .get(target)
                    .ok()
                    .map(|(_, _, position, _)| (target, *position))
            })
            .or_else(|| {
                victims
                    .iter()
                    .filter(|(_, victim_player, _, health)| {
                        victim_player.0 != owner.0 && health.current > 0.0
                    })
                    .min_by(|a, b| {
                        let key_a = crate::core::units::target_preference_key(
                            a.2 .0.distance(position.0),
                            a.0,
                        );
                        let key_b = crate::core::units::target_preference_key(
                            b.2 .0.distance(position.0),
                            b.0,
                        );
                        key_a
                            .0
                            .total_cmp(&key_b.0)
                            .then_with(|| key_a.1.cmp(&key_b.1).then_with(|| key_a.2.cmp(&key_b.2)))
                    })
                    .map(|(target, _, target_position, _)| (target, *target_position))
            })
        else {
            continue;
        };
        let mut entity_commands = commands.entity(entity);
        let cells = target_cells_for(
            |point| navigation.world_to_cell(point),
            None,
            target_position.0,
        );
        if rect_distance(position.0, &cells) > combat.range_for(unit.kind) {
            let ctx = make_approach_context(
                &navigation,
                position.0,
                cells,
                victims
                    .iter()
                    .filter_map(|(_, _, other, _)| (other.0 != position.0).then_some(other)),
                combat.range_for(unit.kind),
                |grid, point| grid.world_to_cell(point),
            );
            let path = approach_path(&ctx, APPROACH_EXPANSION_BOUND);
            if path.is_empty() {
                // Unreachable: no point holding a permanent order the enemy
                // can never execute; it will re-evaluate next tick.
                continue;
            }
            entity_commands.insert(MovePath(path.into()));
        } else {
            entity_commands.remove::<MovePath>();
        }
        // A unit that already holds this exact live order keeps its cooldown
        // clock (`AttackCooldown::default()` means "ready"), so the AI
        // refreshing routing every tick must not reset it — otherwise the
        // enemy would strike on *every* tick instead of once per cooldown.
        let keep_clock = already_ordered;
        if keep_clock {
            entity_commands.insert(AttackOrder { target });
        } else {
            entity_commands.insert((AttackOrder { target }, AttackCooldown::default()));
        }
        entity_commands.insert(Attacking);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::units::KindCombat;

    /// No-op marker system whose only purpose is to register something on the
    /// `FixedMain` schedule in the test app, which enables Bevy's fixed-step
    /// catch-up loop (`RunFixedMainLoop`). Without any system on `FixedMain`,
    /// `MinimalPlugins` would run zero fixed ticks per update.
    fn enable_fixed_step() {}

    /// Nominal fixed-timestep duration in seconds. The virtual clock is
    /// advanced by exactly this amount per `step()`, so every call runs one
    /// full production `FixedUpdate` tick — fully deterministic.
    const DT: f32 = 0.05;

    /// Small walkable open area so units can path and approach targets.
    const TEST_MAP: u32 = 64;

    /// Build a bare Bevy app that runs exactly the production schedules
    /// (same systems, same ordering) without any rendering, window or real
    /// clock. Time uses a virtual clock driven by `ManualDuration(DT)`: every
    /// `app.update()` — including the internal frames that run `Startup` and
    /// the first `Update` — has a delta of exactly one timestep, so each
    /// update executes precisely one `FixedMain` tick. The harness's priming
    /// update (see `TestApp::with_combat`) absorbs the startup frame; from
    /// then on every explicit `step()` is exactly one tick, fully
    /// deterministic and independent of host wall-clock time.
    fn combat_test_app(combat: CombatDefinition) -> App {
        let mut app = App::new();
        // `MinimalPlugins` already provides `TimePlugin`, so only the virtual
        // clock and the manual update strategy are added here. Without a
        // strategy the virtual clock advances by wall-clock time, which makes
        // fixed ticks (and cooldowns) non-deterministic in tests. The delta
        // is *set* to exactly one timestep per update through
        // `ManualDuration` below — never merely accumulated on top of the
        // real elapsed time.
        //
        // The fixed-step driver must also be enabled explicitly: with
        // `MinimalPlugins` the `FixedMain` schedule is disabled until a
        // `run_schedule_independent` system or an explicit enablement turns
        // it on, and `app.update()` would then run *zero* fixed ticks —
        // freezing movement, combat and death systems in tests.
        app.add_plugins(MinimalPlugins)
            .insert_resource(Time::<Virtual>::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                SIM_STEP_SECONDS,
            )));
        // The fixed-step driver itself must be enabled explicitly: with
        // `MinimalPlugins` the `FixedMain` schedule never runs until a
        // system is registered on it, which would freeze movement, combat
        // and death systems in tests. Registering the (no-op) enablement
        // marker on `FixedMain` turns the catch-up loop on; the timestep is
        // pinned to `DT` below so no accumulator drift builds up.
        app.add_systems(FixedMain, enable_fixed_step);
        app.insert_resource(SimTimestep(SIM_STEP_SECONDS as f32))
            .insert_resource(open_grid())
            .insert_resource(combat)
            .init_resource::<PlayerEconomy>()
            .init_resource::<ShedPlacement>()
            .init_resource::<ShedDefinition>()
            .add_event::<PlayerCommand>()
            // Mirror the production schedule's ordering intent: player
            // commands and AI orders must take effect *before* the fixed
            // step's movement/combat systems run within the same tick.
            // Production achieves this by scheduling both appliers first on
            // `FixedMain`. In the test app they run on `Update`, so their
            // deferred writes flush at the end of that update — which is
            // precisely when the next update begins, before its FixedMain
            // systems run. The priming update absorbs the startup frame, so
            // from then on every explicit `step()` is exactly one tick and
            // every `send()` takes effect on the following tick.
            .add_systems(
                Update,
                (
                    apply_player_commands,
                    opponent_ai_system.after(apply_player_commands),
                ),
            )
            .add_systems(
                FixedMain,
                (
                    movement_system,
                    harvest_system.after(movement_system),
                    construction_system.after(harvest_system),
                    attack_system.after(construction_system),
                    death_system.after(attack_system),
                ),
            );
        // Run at exactly the nominal timestep so no accumulator drift builds
        // up between steps.
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep_seconds(DT as f64);
        app
    }

    fn open_grid() -> NavigationGrid {
        NavigationGrid::open_field(TEST_MAP)
    }

    struct TestApp {
        app: App,
    }

    impl TestApp {
        fn new() -> Self {
            Self::with_combat(CombatDefinition::default())
        }

        fn with_combat(combat: CombatDefinition) -> Self {
            let mut app = combat_test_app(combat);
            // Prime the loop once. With `TimeUpdateStrategy::ManualDuration`
            // every update has a delta of exactly one timestep, so this first
            // `app.update()` runs one full fixed tick; tests account for it in
            // their step budgets (e.g. "strike after two steps" = prime + 1).
            app.update();
            TestApp { app }
        }

        fn spawn_unit(
            &mut self,
            kind: UnitKind,
            owner: u8,
            position: Vec2,
            selected: bool,
        ) -> Entity {
            let entity = self
                .app
                .world_mut()
                .spawn((
                    Unit { kind },
                    Player(owner),
                    Health {
                        current: kind.max_hp(),
                        max: kind.max_hp(),
                    },
                    MovePath::default(),
                    WorldPosition(position),
                ))
                .id();
            if selected {
                self.app.world_mut().entity_mut(entity).insert(Selected);
            }
            entity
        }

        fn send(&mut self, command: PlayerCommand) {
            self.app.world_mut().send_event(command);
        }

        /// Advance the simulation by exactly one fixed step of `DT` seconds.
        /// The time plugin runs with `TimeUpdateStrategy::ManualDuration(DT)`,
        /// so each `app.update()` produces a virtual delta of precisely `DT`
        /// and the fixed-step driver executes exactly one `FixedMain` tick.
        /// Pending events flush through `apply_player_commands` on `Update`
        /// before the fixed combat systems run, matching production ordering.
        fn step(&mut self) {
            self.app.update();
        }

        /// Attach an attack order to `attacker` against `target` *without*
        /// running any simulation tick, mirroring the component set that
        /// `apply_player_commands` inserts for an in-range attack (order,
        /// ready cooldown, marker). Tests use this to assert properties of
        /// freshly issued orders — e.g. that a move command cancels them —
        /// before the first tick has had a chance to consume or act on them.
        fn force_attack_order(&mut self, attacker: Entity, target: Entity) {
            self.app.world_mut().entity_mut(attacker).insert((
                AttackOrder { target },
                AttackCooldown::default(),
                Attacking,
            ));
        }

        fn steps(&mut self, count: usize) {
            for _ in 0..count {
                self.step();
            }
        }

        fn health(&self, entity: Entity) -> Option<f32> {
            self.app
                .world()
                .get::<Health>(entity)
                .map(|health| health.current)
        }

        fn has_attack_order(&self, entity: Entity) -> bool {
            self.app.world().get::<AttackOrder>(entity).is_some()
        }

        fn attack_target(&self, entity: Entity) -> Option<Entity> {
            self.app
                .world()
                .get::<AttackOrder>(entity)
                .map(|order| order.target)
        }

        fn cooldown(&self, entity: Entity) -> Option<f32> {
            self.app.world().get::<AttackCooldown>(entity).map(|c| c.0)
        }

        fn position(&self, entity: Entity) -> Option<Vec2> {
            self.app.world().get::<WorldPosition>(entity).map(|p| p.0)
        }

        fn set_position(&mut self, entity: Entity, position: Vec2) {
            self.app
                .world_mut()
                .entity_mut(entity)
                .insert(WorldPosition(position));
            // A teleported unit may now stand nowhere near its queued route;
            // drop any stale path so movement does not drag it back.
            if let Some(mut path) = self.app.world_mut().get_mut::<MovePath>(entity) {
                path.0.clear();
            }
        }

        fn set_health(&mut self, entity: Entity, current: f32) {
            let max = self.health(entity).unwrap_or(current);
            self.app
                .world_mut()
                .entity_mut(entity)
                .insert(Health { current, max });
        }

        fn mark_dead(&mut self, entity: Entity) {
            self.app.world_mut().entity_mut(entity).insert(Dead);
        }

        fn despawn(&mut self, entity: Entity) {
            self.app.world_mut().despawn(entity);
        }

        fn alive_count(&mut self) -> usize {
            self.app
                .world_mut()
                .query_filtered::<Entity, With<Unit>>()
                .iter(self.app.world())
                .count()
        }
    }

    #[test]
    fn damage_kills_at_zero_health_and_despawns_the_unit() {
        let mut test = TestApp::new();
        let attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Peasant, 1, Vec2::X * 10.0, false);

        test.send(PlayerCommand::Attack(target));
        // Step 1 applies the order and walks the attacker into range; step 2
        // lands hit one once the cooldown tick has elapsed.
        test.steps(2);
        assert_eq!(test.health(target), Some(40.0 - 20.0));
        assert!(test.alive_count() == 2, "one hit does not kill");

        // The peasant is at 20 HP: it takes until the third strike (roughly
        // two full cooldowns after the first) to reach zero and despawn.
        test.steps(60);
        assert!(
            test.health(target).is_none(),
            "dead units are removed by the fixed-step sim"
        );
        assert!(
            !test.has_attack_order(attacker),
            "orders against dead targets drop"
        );
    }

    #[test]
    fn attacks_only_land_within_range() {
        let mut test = TestApp::new();
        // Warrior melee range is 30.0; place the target just outside it, in a
        // spot no approach route can reach (see `blocked_grid`), so the unit
        // cannot walk into range and "no damage" really tests the range rule.
        let attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 40.0, false);

        test.send(PlayerCommand::Attack(target));
        test.steps(60); // three full cooldown periods while out of reach
        assert_eq!(test.health(target), Some(100.0), "no damage beyond range");
        assert!(
            !test.has_attack_order(attacker),
            "an unreachable attack position rejects the order"
        );

        // Teleport into range: the next eligible step must deal damage.
        test.set_position(attacker, Vec2::X * 20.0);
        test.send(PlayerCommand::Attack(target));
        test.steps(2);
        assert_eq!(test.health(target), Some(80.0), "damage once in range");
    }

    #[test]
    fn attacks_follow_the_configured_cooldown() {
        let mut test = TestApp::new();
        let attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 10.0, false);

        test.send(PlayerCommand::Attack(target));
        // The order attaches when the command's deferred writes flush, at
        // the end of the update that consumes the event; the fixed combat
        // systems of that same update never see it, so the first strike
        // lands on the *second* step after `send`. Step 2: first hit
        // (100 -> 80, target sits inside melee range and fresh cooldowns
        // start ready). The strike sets the clock to `cooldown -
        // COOLDOWN_PHASE`; each later tick decays it by one step before the
        // ready check, so exactly `cooldown / dt` ticks elapse between
        // strikes: the second hit lands 20 ticks after the first.
        test.steps(2); // step 1: order lands, step 2: first strike
        assert_eq!(test.health(target), Some(80.0));
        assert!(test.cooldown(attacker).is_some());

        // Half a cooldown before the window closes: still only one hit.
        test.steps(29); // 31 steps total, 19 ticks into the 20-tick cooldown
        assert_eq!(test.health(target), Some(80.0), "cooldown gates strikes");

        // One more tick takes the clock to zero: the second hit lands here.
        test.step(); // tick 20 after the first strike
        assert_eq!(test.health(target), Some(60.0));
    }

    #[test]
    fn cooldown_scale_is_data_driven() {
        let mut test = TestApp::with_combat(CombatDefinition {
            cooldown_scale: 2.0,
            ..default()
        });
        let _attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 10.0, false);

        test.send(PlayerCommand::Attack(target));
        // The order attaches when the first `update()` after `send()` flushes
        // its deferred writes; the *next* update's fixed tick is the first
        // that sees the order and strikes (fresh cooldowns start ready), so
        // health is 80 after two steps — same as with the default scale.
        test.steps(2);
        assert_eq!(test.health(target), Some(80.0));
        test.steps(20); // one default (1 s) cooldown would have allowed a second hit
        assert_eq!(
            test.health(target),
            Some(80.0),
            "scaled cooldown halves rate"
        );
        // The doubled cooldown is 2 s = 40 ticks; the strike cadence is
        // exactly `cooldown / dt` ticks, so the second hit lands on step 42.
        test.steps(19); // still cooling down at 40 ticks after the first hit
        assert_eq!(test.health(target), Some(80.0));
        test.step(); // tick 42: the doubled cooldown (2 s) now allows the hit
        assert_eq!(test.health(target), Some(60.0));
    }

    #[test]
    fn per_kind_damage_override_changes_lethality() {
        let mut test = TestApp::with_combat(CombatDefinition {
            warrior: KindCombat {
                damage: Some(40.0),
                ..default()
            },
            ..default()
        });
        let _attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Peasant, 1, Vec2::X * 10.0, false);

        test.send(PlayerCommand::Attack(target));
        test.steps(4); // one overridden 40-damage strike kills the 40 HP peasant
        assert!(
            test.health(target).is_none(),
            "a single lethal blow must kill and despawn"
        );
    }

    #[test]
    fn move_order_cancels_the_attack_order() {
        let mut test = TestApp::new();
        let attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        // Just outside the warrior's 30.0 melee range: no strike can land
        // before the cancel, so this proves the order itself is dropped.
        let target = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 35.0, false);

        // Attach the order directly (same components `apply_player_commands`
        // inserts for an issued attack) so the assertion below is about the
        // cancel rule alone, not about when event-driven orders materialize.
        test.force_attack_order(attacker, target);
        assert!(test.has_attack_order(attacker));

        test.send(PlayerCommand::MoveSelected(Vec2::new(32.0, -8.0)));
        test.step();
        assert!(
            !test.has_attack_order(attacker),
            "move cancels attack order"
        );
        test.steps(40);
        assert_eq!(
            test.health(target),
            Some(100.0),
            "cancelled attacker never strikes"
        );
    }

    #[test]
    fn invalid_targets_are_rejected_or_dropped() {
        let mut test = TestApp::new();
        let attacker = test.spawn_unit(UnitKind::Warrior, 0, Vec2::ZERO, true);
        let friendly = test.spawn_unit(UnitKind::Archer, 0, Vec2::X * 10.0, false);

        // Friendly fire and non-unit entities are refused outright.
        test.send(PlayerCommand::Attack(friendly));
        test.send(PlayerCommand::Attack(Entity::from_raw(9_999)));
        test.steps(4);
        assert!(
            !test.has_attack_order(attacker),
            "invalid orders never attach"
        );
        assert_eq!(test.health(friendly), Some(60.0));

        // An order whose target despawns mid-combat is dropped cleanly.
        let victim = test.spawn_unit(UnitKind::Peasant, 1, Vec2::X * 10.0, false);
        test.send(PlayerCommand::Attack(victim));
        test.step();
        assert_eq!(test.attack_target(attacker), Some(victim));
        test.despawn(victim);
        test.step();
        assert!(
            !test.has_attack_order(attacker),
            "stale orders drop when target vanishes"
        );
    }

    #[test]
    fn non_attacking_units_ignore_attack_orders() {
        let mut test = TestApp::new();
        let peasant = test.spawn_unit(UnitKind::Peasant, 0, Vec2::ZERO, true);
        let target = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 10.0, false);

        test.send(PlayerCommand::Attack(target));
        test.steps(10);
        assert!(!test.has_attack_order(peasant), "peasants cannot fight");
        assert_eq!(test.health(target), Some(100.0));
    }

    #[test]
    fn opponent_ai_attacks_back_and_retargets_after_death() {
        let mut test = TestApp::new();
        // Archer (range 90) vs enemy Warrior (melee 30): the enemy picks the
        // closest player-0 unit on its own and starts attacking it.
        let archer = test.spawn_unit(UnitKind::Archer, 0, Vec2::ZERO, false);
        let enemy = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 25.0, false);

        test.steps(2); // AI assigns the order on the first step, strikes after cooldown
        assert_eq!(test.attack_target(enemy), Some(archer));
        assert_eq!(test.health(archer), Some(60.0 - 20.0));

        // Kill the archer through the sim; the enemy must drop the stale order.
        test.set_health(archer, 0.0);
        test.mark_dead(archer);
        test.steps(3);
        assert!(
            test.health(archer).is_none(),
            "death_system removes the corpse"
        );
        assert_ne!(
            test.attack_target(enemy),
            Some(archer),
            "AI drops orders against dead targets"
        );
        assert_eq!(test.alive_count(), 1);
    }

    #[test]
    fn opponent_ai_moves_into_range_of_far_targets() {
        let mut test = TestApp::new();
        let victim = test.spawn_unit(UnitKind::Peasant, 0, Vec2::ZERO, false);
        // The 64x64 test field spans roughly x in [-504, 504], so a target at
        // world x = 380 is far outside the warrior's 30.0 melee range yet
        // still inside the map: the AI must assign the order *and* queue an
        // approach route toward it instead of rejecting the attack outright.
        let enemy = test.spawn_unit(UnitKind::Warrior, 1, Vec2::X * 380.0, false);

        // The AI's order arrives through deferred commands, which only become
        // visible to direct world queries after the *next* `update()` flushes
        // them: step 1 runs the AI system that assigns the order, step 2
        // makes it observable (and proves it stuck).
        test.steps(2);
        assert_eq!(test.attack_target(enemy), Some(victim));
        let moved = test.position(enemy).unwrap();
        test.steps(20);
        assert!(
            test.position(enemy).unwrap().distance(moved) > 1.0,
            "enemy walks toward its target"
        );
    }
}
