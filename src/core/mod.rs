//! Core game types shared by simulation and rendering layers.

pub mod conf;
pub mod datamanifest;
pub mod gamestate;
pub mod map;
pub mod mapfile;
pub mod scheme;
pub mod units;

use bevy::prelude::*;

/// Game-wide configuration resources.
#[derive(Resource, Debug)]
pub struct GameConfig {
    pub tile_size: f32,
    pub map_width: u32,
    pub map_height: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            tile_size: 1.0,
            map_width: 64,
            map_height: 64,
        }
    }
}

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameConfig>()
            .init_resource::<map::NavigationGrid>()
            .init_resource::<units::PlayerEconomy>()
            .init_resource::<units::ShedDefinition>()
            .init_resource::<units::CombatDefinition>()
            .init_resource::<units::ShedPlacement>()
            .add_plugins(gamestate::DataPlugin);
    }
}
