//! Rust Oberon — open-source RTS inspired by Dark Oberon.
//! Architecture: Bevy ECS with strict sim/render separation.

mod core;
mod render;
mod sim;

use bevy::prelude::*;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("--validate-content") {
        let root = args
            .next()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/do"));
        match core::validate::validate_content(&root) {
            Ok(count) => println!("Content validation passed: {count} maps and schemes checked."),
            Err(errors) => {
                for error in &errors {
                    eprintln!("{error}");
                }
                eprintln!("Content validation failed with {} error(s).", errors.len());
                std::process::exit(1);
            }
        }
        return;
    }

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
