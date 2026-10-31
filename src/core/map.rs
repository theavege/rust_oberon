//! Navigation data derived from the imported map's Earth surface.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use bevy::prelude::*;

use crate::core::{
    mapfile::MapFile,
    scheme::{Scheme, TerrainProps},
};

#[derive(Resource, Debug, Clone, Default)]
pub struct NavigationGrid {
    pub width: u32,
    pub height: u32,
    walkable: Vec<bool>,
    reserved: Vec<bool>,
}

impl NavigationGrid {
    /// Fully walkable grid of `size` x `size` cells (no reserved footprints).
    /// Used by simulation tests that need pathing without a real map file.
    pub fn open_field(size: u32) -> Self {
        Self {
            width: size,
            height: size,
            walkable: vec![true; (size * size) as usize],
            reserved: vec![false; (size * size) as usize],
        }
    }
    /// Build walkability from the scheme's Earth surface. Sea, unmoveable and
    /// non-existent terrain types block units; other Earth terrain is usable.
    pub fn from_map(map: &MapFile, scheme: &Scheme) -> Self {
        let Some(surface) = map.surfaces.get(1) else {
            return Self::default();
        };
        let terrain_types = scheme.segments.get(1).map(|segment| &segment.terrains);
        let walkable: Vec<bool> = surface
            .iter()
            .map(|&terrain_id| {
                terrain_types
                    .and_then(|types| types.get(terrain_id as usize))
                    .is_some_and(terrain_is_walkable)
            })
            .collect();

        let reserved = vec![false; walkable.len()];
        Self {
            width: map.width,
            height: map.height,
            walkable,
            reserved,
        }
    }

    pub fn is_walkable(&self, cell: IVec2) -> bool {
        self.is_terrain_walkable(cell) && !self.reserved[self.cell_index(cell)]
    }

    pub fn is_terrain_walkable(&self, cell: IVec2) -> bool {
        if cell.x < 0 || cell.y < 0 || cell.x >= self.width as i32 || cell.y >= self.height as i32 {
            return false;
        }
        self.walkable[self.cell_index(cell)]
    }

    pub fn can_reserve_footprint(&self, origin: IVec2, size: IVec2) -> bool {
        size.x > 0
            && size.y > 0
            && (origin.y..origin.y + size.y)
                .all(|y| (origin.x..origin.x + size.x).all(|x| self.is_walkable(IVec2::new(x, y))))
    }

    pub fn reserve_footprint(&mut self, origin: IVec2, size: IVec2) -> bool {
        if !self.can_reserve_footprint(origin, size) {
            return false;
        }
        for y in origin.y..origin.y + size.y {
            for x in origin.x..origin.x + size.x {
                let index = self.cell_index(IVec2::new(x, y));
                self.reserved[index] = true;
            }
        }
        true
    }

    pub(crate) fn cell_index(&self, cell: IVec2) -> usize {
        (cell.y as u32 * self.width + cell.x as u32) as usize
    }

    pub fn world_to_cell(&self, world: Vec2) -> IVec2 {
        // Inverse of the isometric cell-center transform in `cell_center`.
        let x_minus_y = world.x / 16.0;
        let x_plus_y = -(world.y + 8.0) / 8.0;
        let x = ((x_minus_y + x_plus_y) * 0.5).round() as i32;
        let y = ((x_plus_y - x_minus_y) * 0.5).round() as i32;
        IVec2::new(x, y)
    }

    pub fn cell_center(cell: IVec2) -> Vec2 {
        Vec2::new(
            16.0 * (cell.x - cell.y) as f32,
            -8.0 * (cell.x + cell.y) as f32 - 8.0,
        )
    }

    /// Closest walkable cell to `requested` within `radius` cells (Chebyshev
    /// distance), searched by increasing ring distance so the first hit is the
    /// nearest one. Used by attack approaches that must stop *near* a target
    /// instead of walking onto it.
    pub fn nearest_walkable_in(&self, requested: IVec2, radius: i32) -> Option<IVec2> {
        for r in 0..=radius.max(0) {
            for dy in -r..=r {
                for dx in -r..=r {
                    // Only visit cells on this ring's border.
                    if dx.abs() != r && dy.abs() != r {
                        continue;
                    }
                    let cell = requested + IVec2::new(dx, dy);
                    if self.is_walkable(cell) {
                        return Some(cell);
                    }
                }
            }
        }
        None
    }

    /// Find a nearby walkable cell for clicks on water or impassable terrain.
    pub fn nearest_walkable(&self, requested: IVec2) -> Option<IVec2> {
        let mut best: Option<(i64, IVec2)> = None;
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                let cell = IVec2::new(x, y);
                if !self.is_walkable(cell) {
                    continue;
                }
                let distance =
                    i64::from(x - requested.x).pow(2) + i64::from(y - requested.y).pow(2);
                if best.is_none_or(|(best_distance, _)| distance < best_distance) {
                    best = Some((distance, cell));
                }
            }
        }
        best.map(|(_, cell)| cell)
    }

    /// A* over the eight neighboring grid cells. Diagonal corner cutting is
    /// disallowed so units cannot pass through the corner of blocked terrain.
    pub fn find_path(&self, start: IVec2, goal: IVec2) -> Option<Vec<IVec2>> {
        if !self.is_walkable(start) || !self.is_walkable(goal) {
            return None;
        }
        if start == goal {
            return Some(Vec::new());
        }

        let index = |cell: IVec2| (cell.y as u32 * self.width + cell.x as u32) as usize;
        let mut frontier = BinaryHeap::new();
        let mut came_from = vec![None; self.walkable.len()];
        let mut cost_so_far = vec![u32::MAX; self.walkable.len()];
        let start_idx = index(start);
        cost_so_far[start_idx] = 0;
        frontier.push(Reverse((heuristic(start, goal), start_idx)));

        const NEIGHBORS: [(i32, i32); 8] = [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (-1, 1),
            (1, -1),
            (1, 1),
        ];

        while let Some(Reverse((_, current_idx))) = frontier.pop() {
            let current = IVec2::new(
                (current_idx as u32 % self.width) as i32,
                (current_idx as u32 / self.width) as i32,
            );
            if current == goal {
                let mut path = Vec::new();
                let mut cursor = current_idx;
                while cursor != start_idx {
                    path.push(IVec2::new(
                        (cursor as u32 % self.width) as i32,
                        (cursor as u32 / self.width) as i32,
                    ));
                    cursor = came_from[cursor]?;
                }
                path.reverse();
                return Some(path);
            }

            for (dx, dy) in NEIGHBORS {
                let next = current + IVec2::new(dx, dy);
                if !self.is_walkable(next) {
                    continue;
                }
                if dx != 0
                    && dy != 0
                    && (!self.is_walkable(current + IVec2::new(dx, 0))
                        || !self.is_walkable(current + IVec2::new(0, dy)))
                {
                    continue;
                }

                let next_idx = index(next);
                let step_cost = if dx == 0 || dy == 0 { 10 } else { 14 };
                let new_cost = cost_so_far[current_idx] + step_cost;
                if new_cost < cost_so_far[next_idx] {
                    cost_so_far[next_idx] = new_cost;
                    came_from[next_idx] = Some(current_idx);
                    frontier.push(Reverse((new_cost + heuristic(next, goal), next_idx)));
                }
            }
        }
        None
    }
}

fn terrain_is_walkable(terrain: &TerrainProps) -> bool {
    !matches!(
        terrain.name.to_ascii_lowercase().as_str(),
        "sea" | "unmoveable" | "nonexist"
    )
}

fn heuristic(a: IVec2, b: IVec2) -> u32 {
    let dx = (a.x - b.x).unsigned_abs();
    let dy = (a.y - b.y).unsigned_abs();
    14 * dx.min(dy) + 10 * (dx.max(dy) - dx.min(dy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_footprint_is_blocked_and_pathfinding_detours() {
        let mut grid = NavigationGrid {
            width: 5,
            height: 5,
            walkable: vec![true; 25],
            reserved: vec![false; 25],
        };
        let origin = IVec2::new(2, 1);
        let size = IVec2::new(1, 3);

        assert!(grid.can_reserve_footprint(origin, size));
        assert!(grid.reserve_footprint(origin, size));
        assert!(!grid.is_walkable(IVec2::new(2, 2)));

        let path = grid.find_path(IVec2::new(0, 2), IVec2::new(4, 2)).unwrap();
        assert!(path
            .iter()
            .all(|cell| cell.x != 2 || !(1..4).contains(&cell.y)));
    }
}
