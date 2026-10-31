//! Project simulation-owned unit positions and selection state into visuals.

use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::core::map::NavigationGrid;
use crate::core::units::{
    Building, ConstructionSite, Dead, Health, Player, PlayerCommand, PlayerEconomy, ResourceNode,
    Selected, ShedDefinition, ShedPlacement, Unit, UnitKind, WorldPosition,
};

/// Marker for unit visuals so we can find unsynced entities.
#[derive(Component)]
pub struct UnitSprite;

pub fn unit_input(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut commands: EventWriter<PlayerCommand>,
    units: Query<(Entity, &Transform, Option<&Player>), With<Unit>>,
    resource_nodes: Query<(Entity, &Transform), With<ResourceNode>>,
) {
    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };

    if buttons.just_pressed(MouseButton::Left) {
        let target = units
            .iter()
            .map(|(entity, transform, _)| {
                (entity, transform.translation.truncate().distance(world_pos))
            })
            .filter(|(_, distance)| *distance <= 22.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(entity, _)| entity);
        commands.write(PlayerCommand::Select(target));
    }
    if buttons.just_pressed(MouseButton::Right) {
        // Right-click on an enemy unit issues an attack order; on a resource
        // node it harvests; otherwise it moves.
        let clicked_unit = units
            .iter()
            .map(|(entity, transform, owner)| {
                (
                    entity,
                    owner.map(|player| player.0),
                    transform.translation.truncate().distance(world_pos),
                )
            })
            .filter(|(_, _, distance)| *distance <= 22.0)
            .min_by(|a, b| a.2.total_cmp(&b.2));
        if let Some((entity, Some(owner), _)) = clicked_unit {
            if owner != 0 {
                commands.write(PlayerCommand::Attack(entity));
                return;
            }
        }
        let resource = resource_nodes
            .iter()
            .map(|(entity, transform)| {
                (entity, transform.translation.truncate().distance(world_pos))
            })
            .filter(|(_, distance)| *distance <= 52.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(entity, _)| entity);
        if let Some(resource) = resource {
            commands.write(PlayerCommand::Harvest(resource));
        } else {
            commands.write(PlayerCommand::MoveSelected(world_pos));
        }
    }
}

pub fn shed_placement_input(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    navigation: Res<NavigationGrid>,
    mut placement: ResMut<ShedPlacement>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut commands: EventWriter<PlayerCommand>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        placement.active = true;
    }
    if keys.just_pressed(KeyCode::Escape) {
        placement.active = false;
        return;
    }
    if placement.active && buttons.just_pressed(MouseButton::Right) {
        placement.active = false;
        return;
    }
    if !placement.active {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };

    let origin = navigation.world_to_cell(world_pos);
    placement.origin = origin;
    if buttons.just_pressed(MouseButton::Left) && placement.valid {
        commands.write(PlayerCommand::PlaceShed(origin));
        placement.active = false;
    }
}

#[derive(Component)]
pub struct ShedPlacementPreview;

pub fn sync_shed_placement_preview(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    shed: Res<ShedDefinition>,
    placement: Res<ShedPlacement>,
    mut previews: Query<(Entity, &mut Sprite, &mut Transform), With<ShedPlacementPreview>>,
) {
    let Ok((entity, mut sprite, mut transform)) = previews.single_mut() else {
        if placement.active {
            let position = footprint_center(placement.origin, shed.footprint);
            commands.spawn((
                Sprite {
                    image: asset_server.load("do/human/shed_build/shed_build1.png"),
                    color: placement_color(placement.valid),
                    custom_size: Some(Vec2::new(100.0, 64.0)),
                    ..default()
                },
                Transform::from_xyz(position.x, position.y, 2.0),
                ShedPlacementPreview,
            ));
        }
        return;
    };

    if !placement.active {
        commands.entity(entity).despawn();
        return;
    }
    let position = footprint_center(placement.origin, shed.footprint);
    sprite.color = placement_color(placement.valid);
    transform.translation.x = position.x;
    transform.translation.y = position.y;
}

fn placement_color(valid: bool) -> Color {
    if valid {
        Color::srgba(0.2, 1.0, 0.25, 0.7)
    } else {
        Color::srgba(1.0, 0.18, 0.14, 0.7)
    }
}

fn footprint_center(origin: IVec2, footprint: IVec2) -> Vec2 {
    let offset = Vec2::new(
        (footprint.x - 1) as f32 / 2.0,
        (footprint.y - 1) as f32 / 2.0,
    );
    NavigationGrid::cell_center(origin)
        + Vec2::new(16.0 * (offset.x - offset.y), -8.0 * (offset.x + offset.y))
}

#[derive(Component)]
pub struct ResourceNodeSprite;

#[derive(Component)]
pub struct EconomyHud;

#[derive(Component)]
pub struct BuildingSprite {
    frame: usize,
    completed: bool,
}

#[derive(Component)]
pub struct ConstructionProgressLabel(Entity);

pub fn sync_shed_sprites(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    buildings: Query<
        (Entity, &WorldPosition, Option<&ConstructionSite>),
        (With<Building>, Without<BuildingSprite>),
    >,
) {
    for (entity, position, site) in &buildings {
        let image = if site.is_some() {
            "do/human/shed_build/shed_build1.png"
        } else {
            "do/human/shed_stay/shed_stay1.png"
        };
        commands.entity(entity).insert((
            Sprite {
                image: asset_server.load(image),
                ..default()
            },
            Transform::from_xyz(position.0.x, position.0.y, 1.2),
            BuildingSprite {
                frame: 0,
                completed: site.is_none(),
            },
        ));
        if site.is_some() {
            commands.spawn((
                Text2d::new("Building Shed 0%"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::WHITE),
                Anchor::Center,
                Transform::from_xyz(position.0.x, position.0.y + 56.0, 2.0),
                ConstructionProgressLabel(entity),
            ));
        }
    }
}

pub fn animate_shed_sprites(
    asset_server: Res<AssetServer>,
    mut buildings: Query<(&ConstructionSite, &mut Sprite, &mut BuildingSprite)>,
) {
    for (site, mut sprite, mut visual) in &mut buildings {
        let progress = (site.elapsed / site.duration).clamp(0.0, 1.0);
        let frame = ((progress * 3.0) as usize).min(2);
        if frame != visual.frame {
            visual.frame = frame;
            sprite.image =
                asset_server.load(format!("do/human/shed_build/shed_build{}.png", frame + 1));
        }
    }
}

pub fn complete_shed_visuals(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut buildings: Query<(&mut Sprite, &mut BuildingSprite), Without<ConstructionSite>>,
    labels: Query<(Entity, &ConstructionProgressLabel)>,
    sites: Query<(), With<ConstructionSite>>,
) {
    for (mut sprite, mut visual) in &mut buildings {
        if !visual.completed {
            sprite.image = asset_server.load("do/human/shed_stay/shed_stay1.png");
            visual.completed = true;
        }
    }
    for (entity, label) in &labels {
        if sites.get(label.0).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

pub fn sync_construction_progress(
    mut labels: Query<(&ConstructionProgressLabel, &mut Text2d)>,
    sites: Query<&ConstructionSite>,
) {
    for (label, mut text) in &mut labels {
        if let Ok(site) = sites.get(label.0) {
            let percent = (100.0 * site.elapsed / site.duration).floor() as u32;
            text.0 = format!("Building Shed {percent}%");
        }
    }
}

pub fn sync_resource_nodes(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    nodes: Query<(Entity, &WorldPosition), (With<ResourceNode>, Without<ResourceNodeSprite>)>,
) {
    for (entity, position) in &nodes {
        commands.entity(entity).insert((
            Sprite {
                image: asset_server.load("do/scheme_plastic/forest_stay/forest_stay1.png"),
                custom_size: Some(Vec2::splat(64.0)),
                ..default()
            },
            Transform::from_xyz(position.0.x, position.0.y, 0.8),
            ResourceNodeSprite,
        ));
    }
}

pub fn sync_resource_transforms(
    mut nodes: Query<(&WorldPosition, &mut Transform), With<ResourceNode>>,
) {
    for (position, mut transform) in &mut nodes {
        transform.translation.x = position.0.x;
        transform.translation.y = position.0.y;
    }
}

pub fn sync_economy_hud(
    economy: Res<PlayerEconomy>,
    mut text: Query<&mut Text2d, With<EconomyHud>>,
    mut last_value: Local<Option<u32>>,
) {
    if *last_value == Some(economy.wood) {
        return;
    }
    *last_value = Some(economy.wood);
    for mut text in &mut text {
        text.0 = format!("Wood: {}", economy.wood);
    }
}

pub fn sync_unit_sprites(
    mut commands: Commands,
    q: Query<(Entity, &Unit, &Player, &WorldPosition), Without<UnitSprite>>,
) {
    for (entity, unit, player, position) in q.iter() {
        // Transform is a presentation copy of the simulation's world position.
        let base = if player.0 == 0 {
            unit.kind.color()
        } else {
            enemy_tint(unit.kind)
        };
        commands.entity(entity).insert((
            Sprite {
                color: base,
                custom_size: Some(Vec2::splat(26.0)),
                ..default()
            },
            UnitSprite,
            HealthBarBase(base),
            Transform::from_xyz(position.0.x, position.0.y, 1.0),
        ));
    }
}

/// Distinguish enemy units by darkening their kind color slightly.
fn enemy_tint(kind: UnitKind) -> Color {
    let mut tinted = kind.color();
    if let Color::Srgba(rgba) = &mut tinted {
        rgba.red *= 0.75;
        rgba.green *= 0.35;
        rgba.blue *= 0.35;
    }
    tinted
}

/// Extract the sRGB channels of a color, falling back to white for other
/// color spaces (all game colors are defined as `Color::srgb`).
fn srgb_channels(color: Color) -> (f32, f32, f32) {
    match color.to_srgba() {
        Srgba::NONE => (1.0, 1.0, 1.0),
        srgba => (srgba.red, srgba.green, srgba.blue),
    }
}

/// Remembers the sprite's non-damage tint so health shading composes correctly.
#[derive(Component)]
pub struct HealthBarBase(pub Color);

/// Clear visual feedback for health and death: damaged units darken toward red
/// as HP drops, dying units flash bright red (before the sim despawns them),
/// and enemy units carry a distinct tint. Selection is shown by size only.
pub fn show_health(
    mut sprites: Query<
        (Entity, &mut Sprite, &HealthBarBase, &Health),
        (With<Unit>, Without<Building>),
    >,
    dead: Query<(), With<Dead>>,
) {
    for (entity, mut sprite, base, health) in &mut sprites {
        if dead.get(entity).is_ok() {
            sprite.color = Color::srgb(1.0, 0.15, 0.1);
            continue;
        }
        let ratio = (health.current / health.max).clamp(0.0, 1.0);
        // Blend from the unit's base color toward red as health drops.
        let (br, bg, bb) = srgb_channels(base.0);
        let damage = 1.0 - ratio;
        sprite.color = Color::srgb(
            br * ratio + 0.85 * damage,
            bg * ratio * 0.7,
            bb * ratio * 0.7,
        );
    }
}

pub fn sync_unit_transforms(mut q: Query<(&WorldPosition, &mut Transform), With<Unit>>) {
    for (position, mut transform) in &mut q {
        transform.translation.x = position.0.x;
        transform.translation.y = position.0.y;
    }
}

pub fn show_selection(mut q: Query<(&Unit, Option<&Selected>, &mut Sprite)>) {
    for (unit, selected, mut sprite) in &mut q {
        // Selection is shown by size only; color carries health feedback
        // (see `show_health`), which runs after this system.
        let _ = unit;
        sprite.custom_size = Some(Vec2::splat(if selected.is_some() { 34.0 } else { 26.0 }));
    }
}

/// Short-lived red flash where a unit died, so death reads clearly on screen.
#[derive(Component)]
pub struct DeathFlash {
    pub remaining: f32,
}

pub fn spawn_death_flashes(
    mut commands: Commands,
    mut removed: RemovedComponents<Unit>,
    positions: Query<&WorldPosition>,
) {
    for entity in removed.read() {
        if let Ok(position) = positions.get(entity) {
            commands.spawn((
                Sprite {
                    color: Color::srgba(1.0, 0.2, 0.1, 0.9),
                    custom_size: Some(Vec2::splat(30.0)),
                    ..default()
                },
                Transform::from_xyz(position.0.x, position.0.y, 1.5),
                DeathFlash { remaining: 0.5 },
            ));
        }
    }
}

pub fn fade_death_flashes(
    time: Res<Time>,
    mut commands: Commands,
    mut flashes: Query<(Entity, &mut DeathFlash, &mut Sprite)>,
) {
    for (entity, mut flash, mut sprite) in &mut flashes {
        flash.remaining -= time.delta_secs();
        let alpha = (flash.remaining / 0.5).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            commands.entity(entity).despawn();
        } else if let Color::Srgba(rgba) = &mut sprite.color {
            rgba.alpha = alpha * 0.9;
        }
    }
}
