//! Unit definitions and shared component types.

use bevy::prelude::*;
use std::collections::VecDeque;

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Peasant,
    Warrior,
    Archer,
}

impl UnitKind {
    pub fn speed(self) -> f32 {
        match self {
            UnitKind::Peasant => 2.0,
            UnitKind::Warrior => 2.5,
            UnitKind::Archer => 2.2,
        }
    }

    pub fn max_hp(self) -> f32 {
        match self {
            UnitKind::Peasant => 40.0,
            UnitKind::Warrior => 100.0,
            UnitKind::Archer => 60.0,
        }
    }

    /// Damage dealt per successful attack (data-driven via `CombatDefinition`).
    pub fn damage(self) -> f32 {
        match self {
            UnitKind::Peasant => 0.0, // peasants do not fight
            UnitKind::Warrior => 20.0,
            UnitKind::Archer => 15.0,
        }
    }

    /// Attack range in isometric world units (one map cell is ~17 px diagonal).
    pub fn attack_range(self) -> f32 {
        match self {
            UnitKind::Peasant => 0.0,
            UnitKind::Warrior => 30.0, // melee
            UnitKind::Archer => 90.0,  // ranged
        }
    }

    /// Seconds between two attacks by the same unit.
    pub fn attack_cooldown(self) -> f32 {
        match self {
            UnitKind::Peasant => 1.0,
            UnitKind::Warrior => 1.0,
            UnitKind::Archer => 1.5,
        }
    }

    /// True when this kind can receive and execute attack orders.
    pub fn can_attack(self) -> bool {
        self.damage() > 0.0
    }

    pub fn color(self) -> Color {
        match self {
            UnitKind::Peasant => Color::srgb(0.9, 0.8, 0.3),
            UnitKind::Warrior => Color::srgb(0.8, 0.2, 0.2),
            UnitKind::Archer => Color::srgb(0.3, 0.7, 0.9),
        }
    }
}

/// Current hit points of a unit.
#[derive(Component, Debug)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    /// Apply damage; returns true when this hit brought the unit to zero.
    pub fn take_damage(&mut self, amount: f32) -> bool {
        if amount <= 0.0 {
            return self.current <= 0.0;
        }
        self.current = (self.current - amount).max(0.0);
        self.current <= 0.0
    }
}

/// Seconds until this unit may attack again (combat cooldown state).
/// A freshly ordered attacker starts at zero — "ready to strike" — which is
/// what `Default` provides; do not change that default, as order assignment
/// relies on it.
#[derive(Component, Debug, Default)]
pub struct AttackCooldown(pub f32);

/// The unit's current attack order and target.
#[derive(Component, Debug)]
pub struct AttackOrder {
    pub target: Entity,
}

/// Marks a unit that has died this step; removed by the cleanup system so the
/// death rule stays in the simulation and out of rendering.
#[derive(Component, Debug)]
pub struct Dead;

/// Deterministic ordering key for combat targets: nearest first (world-space
/// distance from the attacker), then lowest entity generation, then lowest
/// index — i.e. earliest-spawned entity first. Shared by every order
/// assignment path (`apply_player_commands` approach routing and
/// `opponent_ai_system`) so ties resolve identically regardless of Bevy's
/// query iteration order or spawn timing.
pub fn target_preference_key(distance: f32, target: Entity) -> (f32, u32, u32) {
    (distance, target.generation(), target.index())
}

/// Grid route generated for the unit's current movement order.
#[derive(Component, Debug, Default)]
pub struct MovePath(pub VecDeque<IVec2>);

#[derive(Component, Debug)]
pub struct ResourceNode {
    pub wood: u32,
}

#[derive(Component, Debug)]
pub struct HarvestOrder {
    pub target: Entity,
    pub elapsed: f32,
}

#[derive(Resource, Debug, Default)]
pub struct PlayerEconomy {
    pub wood: u32,
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct ShedDefinition {
    pub footprint: IVec2,
    pub wood_cost: u32,
    pub construction_seconds: f32,
}

/// Per-kind combat overrides. `None` keeps the `UnitKind` default; concrete
/// values replace it, so content data can retune one unit type at a time.
#[derive(Clone, Copy, Debug, Default)]
pub struct KindCombat {
    pub damage: Option<f32>,
    pub range: Option<f32>,
    pub cooldown: Option<f32>,
}

/// Data-driven combat tuning. The per-kind damage/range/cooldown defaults come
/// from `UnitKind`; this resource lets those rules be overridden (and later,
/// loaded from content data) without touching simulation code.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CombatDefinition {
    /// Multiplier applied to every attack's damage.
    pub damage_scale: f32,
    /// Multiplier applied to every unit's attack cooldown.
    pub cooldown_scale: f32,
    /// Per-unit-kind overrides applied after the global scales.
    pub peasant: KindCombat,
    pub warrior: KindCombat,
    pub archer: KindCombat,
}

impl Default for CombatDefinition {
    fn default() -> Self {
        Self {
            damage_scale: 1.0,
            cooldown_scale: 1.0,
            peasant: KindCombat::default(),
            warrior: KindCombat::default(),
            archer: KindCombat::default(),
        }
    }
}

impl CombatDefinition {
    fn overrides(&self, kind: UnitKind) -> KindCombat {
        match kind {
            UnitKind::Peasant => self.peasant,
            UnitKind::Warrior => self.warrior,
            UnitKind::Archer => self.archer,
        }
    }

    pub fn damage_for(&self, kind: UnitKind) -> f32 {
        self.overrides(kind)
            .damage
            .unwrap_or_else(|| kind.damage() * self.damage_scale)
    }

    pub fn range_for(&self, kind: UnitKind) -> f32 {
        self.overrides(kind)
            .range
            .unwrap_or_else(|| kind.attack_range())
    }

    pub fn cooldown_for(&self, kind: UnitKind) -> f32 {
        self.overrides(kind)
            .cooldown
            .unwrap_or_else(|| kind.attack_cooldown() * self.cooldown_scale)
    }

    /// True when this kind can fight under the current definitions: positive
    /// damage and a positive range, so a zero-damage or zero-range override
    /// removes a unit from combat even if its kind normally attacks.
    pub fn can_attack(&self, kind: UnitKind) -> bool {
        self.damage_for(kind) > 0.0 && self.range_for(kind) > 0.0
    }
}

impl Default for ShedDefinition {
    fn default() -> Self {
        Self {
            footprint: IVec2::new(2, 2),
            wood_cost: 12,
            construction_seconds: 8.0,
        }
    }
}

#[derive(Component, Debug)]
pub struct Building {
    pub owner: u8,
    pub footprint: IVec2,
}

#[derive(Component, Debug)]
pub struct ConstructionSite {
    pub elapsed: f32,
    pub duration: f32,
}

#[derive(Resource, Debug, Default)]
pub struct ShedPlacement {
    pub active: bool,
    pub origin: IVec2,
    pub valid: bool,
}

/// Unit position in isometric world coordinates, owned by the simulation.
#[derive(Component, Debug, Clone, Copy)]
pub struct WorldPosition(pub Vec2);

/// Player ownership (0 = human, 1 = AI for now).
#[derive(Component, Debug)]
pub struct Player(pub u8);

/// Marks an entity as a selectable unit with a kind.
#[derive(Component, Debug)]
pub struct Unit {
    pub kind: UnitKind,
}

/// Marker for units that currently hold an attack order. `attack_system`
/// maintains it every tick (adding and removing in the same system), so the
/// scheduler can prove its disjoint mutable queries never overlap: attackers
/// carry this marker, pure victims do not.
#[derive(Component, Debug)]
pub struct Attacking;

#[derive(Component, Debug)]
pub struct Selected;

#[derive(Event, Debug, Clone, Copy)]
pub enum PlayerCommand {
    Select(Option<Entity>),
    MoveSelected(Vec2),
    Harvest(Entity),
    PlaceShed(IVec2),
    /// Order every selected attack-capable unit to attack the target unit.
    Attack(Entity),
}
