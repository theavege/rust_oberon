//! Versioned local save snapshots. Disk parsing and full validation happen
//! before any live simulation entities are replaced.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::Path;

use crate::core::{
    gamestate::GameState,
    map::{NavigationGrid, VisibilityGrid},
    units::{
        Building, ConstructionSite, Health, MovePath, Player, PlayerEconomy, ResourceNode, Unit,
        UnitKind, WorldPosition,
    },
};

const SAVE_VERSION: u32 = 1;
const SAVE_PATH: &str = "saves/latest.ron";
const MAX_SAVE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ENTITIES: usize = 10_000;
const MAX_RESOURCE: u32 = 10_000_000;

#[derive(Event, Clone, Copy, Debug)]
pub enum SaveRequest {
    Save,
    Load,
}

#[derive(Resource, Default, Debug)]
pub struct SaveStatus {
    pub message: String,
    pub revision: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    map: String,
    wood: u32,
    explored: Vec<bool>,
    entities: Vec<SavedEntity>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedMovePath {
    path: Vec<[i32; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
enum SavedEntity {
    Unit {
        kind: SavedUnitKind,
        owner: u8,
        position: [f32; 2],
        hp: f32,
        max_hp: f32,
        move_path: Option<SavedMovePath>,
    },
    Building {
        owner: u8,
        origin: [i32; 2],
        footprint: [i32; 2],
        construction: Option<SavedConstruction>,
    },
    Resource {
        wood: u32,
        position: [f32; 2],
    },
}

#[derive(Debug, Serialize, Deserialize)]
enum SavedUnitKind {
    Peasant,
    Warrior,
    Archer,
}

impl From<UnitKind> for SavedUnitKind {
    fn from(kind: UnitKind) -> Self {
        match kind {
            UnitKind::Peasant => Self::Peasant,
            UnitKind::Warrior => Self::Warrior,
            UnitKind::Archer => Self::Archer,
        }
    }
}

impl From<SavedUnitKind> for UnitKind {
    fn from(kind: SavedUnitKind) -> Self {
        match kind {
            SavedUnitKind::Peasant => Self::Peasant,
            SavedUnitKind::Warrior => Self::Warrior,
            SavedUnitKind::Archer => Self::Archer,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SavedConstruction {
    elapsed: f32,
    duration: f32,
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SaveRequest>()
            .init_resource::<SaveStatus>()
            .add_systems(Update, process_save_requests);
    }
}

#[allow(clippy::too_many_arguments)]
fn process_save_requests(
    mut requests: EventReader<SaveRequest>,
    mut commands: Commands,
    game: Option<Res<GameState>>,
    mut navigation: ResMut<NavigationGrid>,
    mut visibility: ResMut<VisibilityGrid>,
    mut economy: ResMut<PlayerEconomy>,
    units: Query<(
        Entity,
        &Unit,
        &Player,
        &Health,
        &WorldPosition,
        Option<&MovePath>,
    )>,
    buildings: Query<(Entity, &Building, &WorldPosition, Option<&ConstructionSite>)>,
    resources: Query<(Entity, &ResourceNode, &WorldPosition)>,
    all_units: Query<Entity, With<Unit>>,
    all_buildings: Query<Entity, With<Building>>,
    all_resources: Query<Entity, With<ResourceNode>>,
    mut status: ResMut<SaveStatus>,
) {
    for request in requests.read() {
        let Some(game) = game.as_ref() else {
            status.message = "Save unavailable: map has not loaded".to_owned();
            status.revision += 1;
            continue;
        };

        match request {
            SaveRequest::Save => {
                let mut entities = Vec::new();
                for (_, unit, owner, health, position, maybe_move_path) in &units {
                    let move_path = maybe_move_path.map(|mp| SavedMovePath {
                        path: mp.0.iter().map(|v| [v.x, v.y]).collect(),
                    });
                    entities.push(SavedEntity::Unit {
                        kind: unit.kind.into(),
                        owner: owner.0,
                        position: [position.0.x, position.0.y],
                        hp: health.current,
                        max_hp: health.max,
                        move_path,
                    });
                }
                for (_, building, position, site) in &buildings {
                    let size = building.footprint;
                    let origin = building_origin(position.0, size);
                    entities.push(SavedEntity::Building {
                        owner: building.owner,
                        origin: [origin.x, origin.y],
                        footprint: [size.x, size.y],
                        construction: site.map(|site| SavedConstruction {
                            elapsed: site.elapsed,
                            duration: site.duration,
                        }),
                    });
                }
                for (_, node, position) in &resources {
                    entities.push(SavedEntity::Resource {
                        wood: node.wood,
                        position: [position.0.x, position.0.y],
                    });
                }
                let snapshot = Snapshot {
                    version: SAVE_VERSION,
                    map: game.map.name.clone(),
                    wood: economy.wood,
                    explored: visibility.explored_cells().to_vec(),
                    entities,
                };
                let result = std::fs::create_dir_all("saves")
                    .and_then(|_| {
                        ron::ser::to_string_pretty(&snapshot, ron::ser::PrettyConfig::default())
                            .map_err(std::io::Error::other)
                    })
                    .and_then(|serialized| std::fs::write(SAVE_PATH, serialized));
                status.message = match result {
                    Ok(()) => format!("Saved game: {SAVE_PATH}"),
                    Err(error) => format!("Save failed: {error}"),
                };
                status.revision += 1;
            }
            SaveRequest::Load => {
                let result = read_snapshot(Path::new(SAVE_PATH))
                    .and_then(|snapshot| validate_snapshot(snapshot, &game.map.name, &navigation));
                let (snapshot, restored_navigation) = match result {
                    Ok(validated) => validated,
                    Err(error) => {
                        status.message = format!("Load failed: {error}");
                        status.revision += 1;
                        continue;
                    }
                };

                for entity in &all_units {
                    commands.entity(entity).despawn();
                }
                for entity in &all_buildings {
                    commands.entity(entity).despawn();
                }
                for entity in &all_resources {
                    commands.entity(entity).despawn();
                }
                for entity in snapshot.entities {
                    match entity {
                        SavedEntity::Unit {
                            kind,
                            owner,
                            position,
                            hp,
                            max_hp,
                            move_path,
                        } => {
                            let kind: UnitKind = kind.into();
                            let mut entity = commands.spawn((
                                Unit { kind },
                                Player(owner),
                                Health {
                                    current: hp,
                                    max: max_hp,
                                },
                                WorldPosition(Vec2::new(position[0], position[1])),
                            ));
                            if let Some(saved_path) = move_path {
                                let path: VecDeque<IVec2> = saved_path
                                    .path
                                    .into_iter()
                                    .map(|p| IVec2::new(p[0], p[1]))
                                    .collect();
                                entity.insert(MovePath(path));
                            }
                        }
                        SavedEntity::Building {
                            owner,
                            origin,
                            footprint,
                            construction,
                        } => {
                            let origin = IVec2::new(origin[0], origin[1]);
                            let size = IVec2::new(footprint[0], footprint[1]);
                            let position = footprint_center(origin, size);
                            let mut entity = commands.spawn((
                                Building {
                                    owner,
                                    footprint: size,
                                },
                                WorldPosition(position),
                            ));
                            if let Some(site) = construction {
                                entity.insert(ConstructionSite {
                                    elapsed: site.elapsed,
                                    duration: site.duration,
                                });
                            }
                        }
                        SavedEntity::Resource { wood, position } => {
                            commands.spawn((
                                ResourceNode { wood },
                                WorldPosition(Vec2::new(position[0], position[1])),
                            ));
                        }
                    }
                }
                *navigation = restored_navigation;
                let _ = visibility.restore_explored(&snapshot.explored);
                economy.wood = snapshot.wood;
                status.message = format!("Loaded game: {SAVE_PATH}");
                status.revision += 1;
            }
        }
    }
}

fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_SAVE_BYTES {
        return Err("save file exceeds the 8 MiB limit".to_owned());
    }
    let contents = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    ron::from_str(&contents).map_err(|error| format!("invalid save syntax: {error}"))
}

fn validate_snapshot(
    snapshot: Snapshot,
    current_map: &str,
    navigation: &NavigationGrid,
) -> Result<(Snapshot, NavigationGrid), String> {
    if snapshot.version != SAVE_VERSION {
        return Err(format!("unsupported save version {}", snapshot.version));
    }
    if snapshot.map != current_map {
        return Err(format!(
            "save is for map '{}', current map is '{current_map}'",
            snapshot.map
        ));
    }
    if snapshot.wood > MAX_RESOURCE || snapshot.entities.len() > MAX_ENTITIES {
        return Err("save exceeds supported resource or entity limits".to_owned());
    }
    if snapshot.explored.len() != (navigation.width * navigation.height) as usize {
        return Err("exploration grid dimensions do not match the current map".to_owned());
    }

    let mut restored_navigation = navigation.clone();
    restored_navigation.clear_reservations();
    for entity in &snapshot.entities {
        match entity {
            SavedEntity::Unit {
                kind: _,
                owner,
                position,
                hp,
                max_hp,
                move_path,
            } => {
                if *owner > 1
                    || !valid_position(*position, &restored_navigation)
                    || !hp.is_finite()
                    || !max_hp.is_finite()
                    || *hp <= 0.0
                    || *max_hp <= 0.0
                    || *hp > *max_hp
                    || *max_hp > 10_000.0
                {
                    return Err("save contains an invalid unit".to_owned());
                }
                if let Some(ref path) = move_path {
                    if path.path.len() > 100 {
                        return Err(
                            "save contains a unit with an excessively long move path".to_owned()
                        );
                    }
                    for point in &path.path {
                        let cell = IVec2::new(point[0], point[1]);
                        if !restored_navigation.is_terrain_walkable(cell) {
                            return Err(
                                "save contains a move path through unwalkable terrain".to_owned()
                            );
                        }
                    }
                }
            }
            SavedEntity::Building {
                owner,
                origin,
                footprint,
                construction,
            } => {
                let origin = IVec2::new(origin[0], origin[1]);
                let size = IVec2::new(footprint[0], footprint[1]);
                if *owner > 1
                    || size.x <= 0
                    || size.y <= 0
                    || size.x > 16
                    || size.y > 16
                    || !restored_navigation.reserve_footprint(origin, size)
                {
                    return Err("save contains an invalid or overlapping building".to_owned());
                }
                if let Some(site) = construction {
                    if !site.elapsed.is_finite()
                        || !site.duration.is_finite()
                        || site.elapsed < 0.0
                        || site.duration <= 0.0
                        || site.elapsed > site.duration
                    {
                        return Err("save contains invalid construction progress".to_owned());
                    }
                }
            }
            SavedEntity::Resource { wood, position } => {
                if *wood > MAX_RESOURCE || !valid_position(*position, &restored_navigation) {
                    return Err("save contains an invalid resource node".to_owned());
                }
            }
        }
    }
    for entity in &snapshot.entities {
        if let SavedEntity::Unit { position, .. } = entity {
            let cell = restored_navigation.world_to_cell(Vec2::new(position[0], position[1]));
            if !restored_navigation.is_walkable(cell) {
                return Err("save places a unit inside a building footprint".to_owned());
            }
        }
    }
    Ok((snapshot, restored_navigation))
}

fn valid_position(position: [f32; 2], navigation: &NavigationGrid) -> bool {
    position[0].is_finite()
        && position[1].is_finite()
        && navigation
            .is_terrain_walkable(navigation.world_to_cell(Vec2::new(position[0], position[1])))
}

fn footprint_center(origin: IVec2, footprint: IVec2) -> Vec2 {
    let offset = Vec2::new(
        (footprint.x - 1) as f32 / 2.0,
        (footprint.y - 1) as f32 / 2.0,
    );
    NavigationGrid::cell_center(origin)
        + Vec2::new(16.0 * (offset.x - offset.y), -8.0 * (offset.x + offset.y))
}

fn building_origin(position: Vec2, footprint: IVec2) -> IVec2 {
    let x_minus_y = position.x / 16.0;
    let x_plus_y = -(position.y + 8.0) / 8.0;
    let center_x = (x_minus_y + x_plus_y) * 0.5;
    let center_y = (x_plus_y - x_minus_y) * 0.5;
    IVec2::new(
        (center_x - (footprint.x - 1) as f32 / 2.0).round() as i32,
        (center_y - (footprint.y - 1) as f32 / 2.0).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> Snapshot {
        let building_origin = IVec2::new(3, 4);
        Snapshot {
            version: SAVE_VERSION,
            map: "Test Map".to_owned(),
            wood: 17,
            explored: vec![true; 64],
            entities: vec![
                SavedEntity::Unit {
                    kind: SavedUnitKind::Archer,
                    owner: 0,
                    position: [
                        NavigationGrid::cell_center(IVec2::new(1, 2)).x,
                        NavigationGrid::cell_center(IVec2::new(1, 2)).y,
                    ],
                    hp: 35.0,
                    max_hp: 60.0,
                    move_path: None,
                },
                SavedEntity::Building {
                    owner: 0,
                    origin: [building_origin.x, building_origin.y],
                    footprint: [2, 2],
                    construction: Some(SavedConstruction {
                        elapsed: 3.5,
                        duration: 8.0,
                    }),
                },
                SavedEntity::Resource {
                    wood: 12,
                    position: [
                        NavigationGrid::cell_center(IVec2::new(6, 6)).x,
                        NavigationGrid::cell_center(IVec2::new(6, 6)).y,
                    ],
                },
            ],
        }
    }

    #[test]
    fn snapshot_round_trips_units_buildings_resources_and_exploration() {
        let snapshot = sample_snapshot();
        let encoded = ron::to_string(&snapshot).unwrap();
        let decoded: Snapshot = ron::from_str(&encoded).unwrap();
        let navigation = NavigationGrid::open_field(8);
        let (validated, restored_navigation) =
            validate_snapshot(decoded, "Test Map", &navigation).unwrap();

        assert_eq!(validated.wood, 17);
        assert_eq!(validated.entities.len(), 3);
        assert_eq!(validated.explored, vec![true; 64]);
        assert!(!restored_navigation.is_walkable(IVec2::new(3, 4)));

        for size in [IVec2::ONE, IVec2::new(2, 2), IVec2::new(3, 2)] {
            let origin = IVec2::new(2, 3);
            assert_eq!(
                building_origin(footprint_center(origin, size), size),
                origin
            );
        }
    }

    #[test]
    fn invalid_entity_data_is_rejected_without_mutating_map_state() {
        let mut snapshot = sample_snapshot();
        snapshot.entities[0] = SavedEntity::Unit {
            kind: SavedUnitKind::Peasant,
            owner: 0,
            position: [f32::NAN, 0.0],
            hp: 30.0,
            max_hp: 40.0,
            move_path: None,
        };
        let navigation = NavigationGrid::open_field(8);

        assert!(validate_snapshot(snapshot, "Test Map", &navigation).is_err());
        assert!(navigation.is_walkable(IVec2::new(3, 4)));
    }

    #[test]
    fn unsupported_save_versions_are_rejected() {
        let mut snapshot = sample_snapshot();
        snapshot.version += 1;

        assert!(validate_snapshot(snapshot, "Test Map", &NavigationGrid::open_field(8)).is_err());
    }

    #[test]
    fn units_cannot_load_inside_a_building_footprint() {
        let mut snapshot = sample_snapshot();
        let cell = NavigationGrid::cell_center(IVec2::new(3, 4));
        snapshot.entities[0] = SavedEntity::Unit {
            kind: SavedUnitKind::Archer,
            owner: 0,
            position: [cell.x, cell.y],
            hp: 35.0,
            max_hp: 60.0,
            move_path: None,
        };

        assert!(validate_snapshot(snapshot, "Test Map", &NavigationGrid::open_field(8)).is_err());
    }
}
