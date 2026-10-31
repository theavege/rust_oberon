//! Top-down 2D camera with WASD pan and mouse-edge scroll hooks (later).

use crate::core::gamestate::GameState;
use crate::render::units_view::EconomyHud;
use bevy::prelude::*;
use bevy::render::camera::ScalingMode;
use bevy::sprite::Anchor;

pub fn spawn_camera(mut commands: Commands, game: Option<Res<GameState>>) {
    let center = game
        .as_ref()
        .map(|gs| GameState::iso_pos(gs.map.width / 2, gs.map.height / 2))
        .unwrap_or(Vec2::new(0.0, -640.0));
    let camera = commands
        .spawn((
            Camera2d,
            // Show a useful portion of the imported map at its native pixel scale.
            Projection::from(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: 900.0,
                },
                ..OrthographicProjection::default_2d()
            }),
            Transform::from_xyz(center.x, center.y, 1000.0),
        ))
        .id();
    commands.entity(camera).with_children(|parent| {
        parent.spawn((
            Text2d::new("Wood: 0"),
            TextFont {
                font_size: 28.0,
                ..default()
            },
            TextColor(Color::WHITE),
            Anchor::TopLeft,
            Transform::from_xyz(-760.0, 420.0, -998.0),
            EconomyHud,
        ));
    });
}

/// Pan the camera with arrow keys / WASD.
pub fn camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Query<&mut Transform, With<Camera2d>>,
) {
    let speed = 600.0 * time.delta_secs().min(1.0 / 30.0);
    let mut dir = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        dir.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        dir.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        dir.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        dir.x += 1.0;
    }
    if dir != Vec2::ZERO {
        for mut transform in q.iter_mut() {
            transform.translation.x += dir.x * speed;
            transform.translation.y += dir.y * speed;
        }
    }
}
