//! Game state resource: loads the original Dark Oberon map + scheme at
//! startup (faithful to `LoadScheme` in doschemes.cpp:771 and `TGameMap::Load`).
//!
//! Loading order mirrors the C original:
//! 1. load the scheme `.dat` texture manifest (`plastic.dat` → group list),
//! 2. parse the `.sch` (materials, segments, fragments; terrain-id remapping),
//! 3. parse the `.map` against the scheme (fragment placement + default fill).

use std::path::{Path, PathBuf};

use bevy::log::{error, info};
use bevy::prelude::*;

use crate::core::datamanifest::Manifest;
use crate::core::map::NavigationGrid;
use crate::core::mapfile::MapFile;
use crate::core::scheme::Scheme;

/// Isometric fragment geometry from the original renderer (domap.h:52–53):
/// a cell is STRAIGHT(32) px wide and DIAGONAL(16) px tall on screen;
/// screen position of cell (x,y) = (16*(x-y), 8*(x+y)).
pub const CELL_W: f32 = 32.0; // STRAIGHT
pub const CELL_H: f32 = 16.0; // DIAGONAL (full rhombus height = 2*DIAGONAL? no: y step per cell is DIAGONAL/2)

#[derive(Resource, Clone)]
pub struct GameState {
    pub scheme: Scheme,
    pub manifest: Manifest,
    pub map: MapFile,
    /// AssetServer-relative directory (`do/scheme_plastic` under `assets/`).
    pub asset_dir: PathBuf,
}

/// Locate the workspace root by walking up from CWD until `rust_oberon/Cargo.toml`
/// is found — lets us reference the same files from `cargo run` (cwd = repo or
/// crate dir) and from tests.
fn find_assets_dir() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        for cand in [dir.join("assets/do"), dir.join("rust_oberon/assets/do")] {
            if cand.exists() {
                return Some(cand);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

impl GameState {
    pub fn load(do_root: &Path, map_name: &str) -> Result<Self, String> {
        crate::core::validate::validate_content(do_root).map_err(|errors| {
            format!(
                "content validation failed before loading '{map_name}':\n{}",
                errors.join("\n")
            )
        })?;

        let disk_asset_dir = do_root.join("scheme_plastic");
        let manifest_src = std::fs::read_to_string(disk_asset_dir.join("manifest.json"))
            .map_err(|e| format!("manifest: {e}"))?;
        let manifest = Manifest::parse(&manifest_src)?;
        // Bevy's default asset root is `assets/`, so runtime paths are
        // relative to it rather than absolute filesystem paths.
        let asset_dir = PathBuf::from("do/scheme_plastic");

        // The .sch/.map originals are vendored under assets/ (verbatim copies
        // from the Dark Oberon GPL repository — see assets/CREDITS.md).
        let sch_candidates = [
            do_root.join("schemes/plastic.sch"),
            do_root.join("../schemes/plastic.sch"),
            PathBuf::from("/tmp/dark-oberon/schemes/plastic.sch"),
        ];
        let sch_path = sch_candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or("plastic.sch not found")?;
        let scheme =
            Scheme::parse(&std::fs::read_to_string(&sch_path).map_err(|e| format!("sch: {e}"))?)?;

        let map_candidates = [
            do_root.join("maps").join(format!("{map_name}.map")),
            do_root.join("../maps").join(format!("{map_name}.map")),
            PathBuf::from("/tmp/dark-oberon/maps").join(format!("{map_name}.map")),
        ];
        let map_path = map_candidates
            .iter()
            .find(|p| p.exists())
            .cloned()
            .ok_or(format!("{map_name}.map not found"))?;
        let map = MapFile::parse(
            &std::fs::read_to_string(&map_path).map_err(|e| format!("map: {e}"))?,
            &scheme,
        )?;

        Ok(Self {
            scheme,
            manifest,
            map,
            asset_dir,
        })
    }

    /// Texture group index as C `ReadTextureGroup` would resolve it.
    pub fn group_index(&self, name: Option<&str>) -> usize {
        match name {
            Some(n) => self
                .manifest
                .groups
                .iter()
                .position(|g| g.name == n)
                .unwrap_or(0),
            None => 0,
        }
    }

    /// Screen-space position of the *origin corner* of cell (x,y), matching
    /// `TTERR_FRAG::Draw`: glTranslatef(-STRAIGHT/2*x + DIAGONAL/2*y, ...) etc.
    /// In DO's coordinate system (y flipped vs GL screen-up), we return world
    /// coords with +y up: top vertex of the fragment rhombus.
    pub fn iso_pos(x: u32, y: u32) -> Vec2 {
        Vec2::new(
            (CELL_W / 2.0) * (x as f32 - y as f32),
            -(CELL_H / 2.0) * (x as f32 + y as f32),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_map_load_uses_content_validation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/do");
        let game = GameState::load(&root, "trial").expect("validated Trial map loads");
        assert_eq!(game.map.width, 80);
        assert_eq!(game.map.height, 80);
    }
}

/// Event to request a map change (campaign transition).
/// The current game state is preserved until the new map passes validation and loads successfully.
#[derive(Event, Debug, Clone)]
pub struct MapChangeRequest {
    pub map_name: String,
}

/// Tracks an in-progress map change to preserve the current state until the new map loads.
#[derive(Resource, Default)]
pub struct MapChangeState {
    /// The previous game state, preserved during map transition.
    previous_game: Option<Box<GameState>>,
    /// The previous navigation grid, preserved during map transition.
    previous_navigation: Option<Box<NavigationGrid>>,
    /// Whether a map change is currently in progress.
    in_progress: bool,
}

pub fn init_game_state(
    mut commands: Commands,
    existing: Option<Res<GameState>>,
    mut attempted: Local<bool>,
) {
    if existing.is_some() || *attempted {
        return;
    }
    *attempted = true;
    let do_root = find_assets_dir().unwrap_or_else(|| PathBuf::from("assets/do"));
    match GameState::load(&do_root, "trial") {
        Ok(gs) => {
            info!(
                "Loaded map '{}' ({}x{}, {} placed fragments, scheme '{}')",
                gs.map.name,
                gs.map.width,
                gs.map.height,
                gs.map.fragments.len(),
                gs.scheme.name
            );
            commands.insert_resource(NavigationGrid::from_map(&gs.map, &gs.scheme));
            commands.insert_resource(gs);
        }
        Err(e) => {
            error!("Failed to load game state: {e}");
        }
    }
}

/// Handles map change requests for campaign transitions.
/// Routes each map change through `GameState::load` and preserves the current
/// game until the replacement map has passed validation and loaded successfully.
pub fn handle_map_change_requests(
    mut requests: EventReader<MapChangeRequest>,
    mut commands: Commands,
    current_game: Option<Res<GameState>>,
    current_navigation: Option<Res<NavigationGrid>>,
    mut change_state: ResMut<MapChangeState>,
) {
    // Only process the first request to avoid overlapping transitions
    let Some(request) = requests.read().next() else {
        return;
    };

    // Skip if already in progress or no current game
    if change_state.in_progress || current_game.is_none() {
        return;
    }

    change_state.in_progress = true;

    // Preserve current state before attempting to load the new map
    if let (Some(game), Some(navigation)) = (current_game.as_ref(), current_navigation.as_ref()) {
        change_state.previous_game = Some(Box::new((*game).clone()));
        change_state.previous_navigation = Some(Box::new((*navigation).clone()));
    }

    let do_root = find_assets_dir().unwrap_or_else(|| PathBuf::from("assets/do"));

    // Attempt to load the new map through GameState::load (includes validation)
    match GameState::load(&do_root, &request.map_name) {
        Ok(new_game) => {
            info!(
                "Map change to '{}' successful ({}x{}, {} fragments, scheme '{}')",
                new_game.map.name,
                new_game.map.width,
                new_game.map.height,
                new_game.map.fragments.len(),
                new_game.scheme.name
            );

            // New map loaded and validated successfully - replace the old state
            let new_navigation = NavigationGrid::from_map(&new_game.map, &new_game.scheme);
            commands.insert_resource(new_game);
            commands.insert_resource(new_navigation);

            // Clear preserved state
            change_state.previous_game = None;
            change_state.previous_navigation = None;
            change_state.in_progress = false;
        }
        Err(e) => {
            error!(
                "Map change to '{}' failed: {}. Previous game state preserved.",
                request.map_name, e
            );
            // On failure, keep the preserved state - the old resources are still in place
            change_state.in_progress = false;
        }
    }
}

pub struct DataPlugin;

impl Plugin for DataPlugin {
    fn build(&self, app: &mut App) {
        // Runs before the render Startup systems (which read GameState).
        app.add_systems(PreUpdate, init_game_state)
            .add_event::<MapChangeRequest>()
            .init_resource::<MapChangeState>()
            .add_systems(Update, handle_map_change_requests);
    }
}
