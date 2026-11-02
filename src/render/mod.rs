//! Rendering layer: turns simulation state into visuals.
//! Reads sim state, never writes it (except view-only resources like camera).

pub mod camera;
pub mod terrain;
pub mod units_view;

use bevy::prelude::*;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(terrain::TerrainRenderPlugin)
            .add_systems(Startup, camera::spawn_camera)
            .add_systems(
                Update,
                (
                    camera::camera_pan,
                    units_view::save_load_input,
                    units_view::sync_save_status,
                    units_view::unit_input,
                    units_view::shed_placement_input,
                    units_view::sync_shed_placement_preview,
                    units_view::sync_shed_sprites,
                    units_view::animate_shed_sprites,
                    units_view::complete_shed_visuals,
                    units_view::sync_construction_progress,
                    units_view::sync_resource_nodes,
                    units_view::sync_resource_transforms,
                    units_view::sync_unit_sprites,
                    units_view::sync_unit_transforms,
                ),
            )
            .add_systems(
                Update,
                (
                    units_view::sync_fog_visibility,
                    units_view::sync_role_labels.after(units_view::sync_fog_visibility),
                    units_view::sync_health_bars.after(units_view::sync_fog_visibility),
                    units_view::spawn_combat_text,
                    units_view::animate_combat_text,
                    units_view::show_selection,
                    units_view::show_health,
                ),
            )
            .add_systems(
                Update,
                (
                    units_view::spawn_death_flashes,
                    units_view::fade_death_flashes,
                    units_view::sync_economy_hud,
                ),
            );
    }
}
