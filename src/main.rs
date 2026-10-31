//! Rust Oberon — open-source RTS inspired by Dark Oberon.
//! Architecture: Bevy ECS with strict sim/render separation.

mod core;
mod render;
mod sim;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Rust Oberon".to_string(),
                resolution: (1280.0_f32, 720.0_f32).into(),
                ..default()
            }),
            ..default()
        }))
        // Simulation steps at a fixed rate for determinism.
        .insert_resource(Time::<Fixed>::from_hz(20.0))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.09, 0.06)))
        .add_plugins(core::CorePlugin)
        .add_plugins(sim::SimPlugin)
        .add_plugins(render::RenderPlugin)
        .run();
}
