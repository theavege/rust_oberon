//! Initial game state: spawn a small starting squad for player 0.

use bevy::prelude::*;

use crate::core::{
    gamestate::{GameState, CELL_H},
    map::NavigationGrid,
    units::{Health, MovePath, Player, ResourceNode, Unit, UnitKind, WorldPosition},
};

pub fn spawn_initial_units(
    mut commands: Commands,
    game: Option<Res<GameState>>,
    navigation: Option<Res<NavigationGrid>>,
) {
    let anchor = game
        .as_ref()
        .and_then(|gs| gs.map.start_points.first())
        .map(|p| (p.x, p.y))
        .unwrap_or((5, 10));
    let starts = [
        (UnitKind::Peasant, (anchor.0, anchor.1)),
        (UnitKind::Warrior, (anchor.0 + 2, anchor.1)),
        (UnitKind::Archer, (anchor.0 + 4, anchor.1)),
    ];
    for (kind, (x, y)) in starts {
        let pos = GameState::iso_pos(x, y) + Vec2::new(0.0, -CELL_H / 2.0);
        commands.spawn((
            Unit { kind },
            Player(0),
            Health {
                current: kind.max_hp(),
                max: kind.max_hp(),
            },
            MovePath::default(),
            WorldPosition(pos),
        ));
    }

    let wood_cells = [
        (anchor.0.saturating_sub(6), anchor.1.saturating_sub(3)),
        (anchor.0.saturating_sub(7), anchor.1.saturating_add(3)),
    ];
    for (x, y) in wood_cells {
        let requested = bevy::math::IVec2::new(x as i32, y as i32);
        let pos = navigation
            .as_ref()
            .and_then(|grid| grid.nearest_walkable(requested))
            .map(NavigationGrid::cell_center)
            .unwrap_or_else(|| GameState::iso_pos(x, y) + Vec2::new(0.0, -CELL_H / 2.0));
        commands.spawn((ResourceNode { wood: 30 }, WorldPosition(pos)));
    }

    // A single enemy warrior (player 1) stands a few cells to the east so the
    // combat slice is playable: select a Warrior or Archer, right-click it.
    let enemy_cell = (anchor.0.saturating_add(8), anchor.1);
    let enemy_pos = GameState::iso_pos(enemy_cell.0, enemy_cell.1) + Vec2::new(0.0, -CELL_H / 2.0);
    commands.spawn((
        Unit {
            kind: UnitKind::Warrior,
        },
        Player(1),
        Health {
            current: UnitKind::Warrior.max_hp(),
            max: UnitKind::Warrior.max_hp(),
        },
        MovePath::default(),
        WorldPosition(enemy_pos),
    ));
}
