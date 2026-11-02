//! Isometric terrain renderer for Dark Oberon maps.
//!
//! Faithful port of `TTERR_FRAG::Draw` / `TMapSegment::Draw` (domap.cpp:85–140,
//! domap.h:494): every placed map fragment is drawn as ONE rhombus sprite at
//! its cell origin; screen position = (STRAIGHT/2*(x-y), -DIAGONAL/2*(x+y))
//! with STRAIGHT=32, DIAGONAL=16 px; sprite size = width*32 × height*16 px.
//!
//! Texture selection mirrors the C loader: a fragment's `tg_id` resolves to a
//! texture-group index in the scheme .dat group order (`GameState::group_index`,
//! first entry of that group); the implicit default fragment always uses group
//! index 0 (doschemes.cpp:495 quirk). Animation variants within a group are
//! chosen pseudo-randomly per instance — DO calls `GetRandomInt(count)` once
//! per fragment at load time (domap.cpp:126); we reproduce that with a small
//! deterministic LCG so runs are reproducible. Animated textures (frames_h/v >
//! 1) advance frames globally on the original timing (anim_ms per full cycle,
//! glgui.cpp TGUI_TEXTURE::DrawFrame row-major slicing).

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::image::{Image, TextureAtlasLayout};
use bevy::log::warn;
use bevy::prelude::*;
use bevy::sprite::Anchor;

use crate::core::gamestate::GameState;
use crate::core::map::VisibilityGrid;

/// One rendered fragment instance.
#[derive(Component)]
struct TerrainFragment {
    segment: usize,
    cell: IVec2,
}

/// Per-texture-frame animation state attached to animated sprites.
#[derive(Component)]
struct FragmentAnim {
    /// Frame rects inside the atlas layout, in display order.
    frames: Vec<usize>,
    anim_ms: u32,
    elapsed: f32,
    current: usize,
}

/// Atlas handle cache keyed by PNG path (single decode pass per file).
#[derive(Resource, Default)]
struct GroupAtlases(HashMap<String, (Handle<Image>, Handle<TextureAtlasLayout>)>);

/// Deterministic variant picker mirroring one GetRandomInt call per fragment
/// in placement order (C used global rand seeded per game; ours is fixed so
/// test/dev runs are stable — see M9 for parity with DO's seed handling).
struct VariantRng(u64);

impl VariantRng {
    fn next(&mut self, n: usize) -> usize {
        // Numerical Recipes LCG constants.
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % n.max(1)
    }
}

/// Load every distinct PNG referenced by the scheme manifest exactly once,
/// building an atlas layout per file (frames laid out left→right, top→bottom,
/// matching TGUI_TEXTURE::DrawFrame grid indexing).
fn load_group_atlases(
    gs: &GameState,
    atlases: &mut GroupAtlases,
    images: &Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
    asset_servers: &AssetServer,
) -> bool {
    let mut all_ready = true;
    for group in &gs.manifest.groups {
        for tex in &group.textures {
            if atlases.0.contains_key(&tex.file) {
                continue;
            }
            let entry = atlases.0.entry(tex.file.clone()).or_insert_with(|| {
                let handle: Handle<Image> =
                    asset_servers.load(asset_path(&gs.asset_dir, &tex.file));
                (handle, Handle::<TextureAtlasLayout>::default())
            });
            // Build the atlas layout once the image has finished decoding.
            if entry.1.is_weak() {
                match asset_servers.get_load_state(&entry.0) {
                    Some(LoadState::Loaded) => {
                        if let Some(img) = images.get(&entry.0) {
                            let sz = img.size();
                            let wpx = sz.x;
                            let hpx = sz.y;
                            let cols = tex.frames_h.max(1) as u32;
                            let rows = tex.frames_v.max(1) as u32;
                            let fw = wpx / cols;
                            let fh = hpx / rows;
                            let mut textures = Vec::new();
                            for r in 0..rows {
                                for c in 0..cols {
                                    textures.push(URect::new(
                                        c * fw,
                                        r * fh,
                                        (c + 1) * fw,
                                        (r + 1) * fh,
                                    ));
                                }
                            }
                            let layout = layouts.add(TextureAtlasLayout {
                                size: UVec2::new(wpx, hpx),
                                textures,
                            });
                            entry.1 = layout;
                        } else {
                            all_ready = false;
                        }
                    }
                    Some(LoadState::Failed(_)) => {
                        warn!("failed to load scheme texture {}", tex.file);
                        atlases.0.remove(&tex.file);
                    }
                    _ => all_ready = false,
                }
            }
        }
    }
    all_ready
}

fn asset_path(dir: &std::path::Path, rel: &str) -> String {
    dir.join(rel).to_string_lossy().replace('\\', "/")
}

/// Spawn one sprite per placed map fragment once GameState and all scheme
/// textures are available. Runs until it succeeds, then removes itself.
pub fn spawn_terrain(
    mut commands: Commands,
    gs: Option<Res<GameState>>,
    images: Res<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    asset_servers: Res<AssetServer>,
    mut atlases: ResMut<GroupAtlases>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(gs) = gs else { return };
    if !load_group_atlases(&gs, &mut atlases, &images, &mut layouts, &asset_servers) {
        return;
    }

    let mut rng = VariantRng(0x9E3779B97F4A7C15);
    for pf in &gs.map.fragments {
        let Some(seg) = gs.scheme.segments.get(pf.segment) else {
            continue;
        };
        let Some(frag) = seg.fragments.get(pf.fid) else {
            continue;
        };
        // Resolve texture group like C ReadTextureGroup: name → index,
        // default fragment (and unresolved names) → group 0.
        let gi = if frag.is_default {
            0
        } else {
            gs.group_index(frag.tg_id.as_deref())
        };
        let Some(group) = gs.manifest.groups.get(gi) else {
            continue;
        };
        if group.textures.is_empty() {
            continue;
        }
        // Pick variant among the group's textures (C: GetRandomInt over the
        // group's texture list when loading the fragment table).
        let vi = rng.next(group.textures.len());
        let tex = &group.textures[vi];
        let Some((img, layout)) = atlases.0.get(&tex.file) else {
            continue;
        };

        let w_px = frag.width as f32 * crate::core::gamestate::CELL_W;
        let h_px = frag.height as f32 * crate::core::gamestate::CELL_H;
        let pos = GameState::iso_pos(pf.x, pf.y);

        if tex.frames_h > 1 || tex.frames_v > 1 {
            let n = (tex.frames_h * tex.frames_v) as usize;
            let frames: Vec<usize> = (0..n).collect();
            commands.spawn((
                Sprite {
                    image: img.clone(),
                    texture_atlas: Some(TextureAtlas {
                        layout: layout.clone(),
                        index: 0,
                    }),
                    custom_size: Some(Vec2::new(w_px, h_px)),
                    anchor: Anchor::TopLeft,
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, 0.0),
                FragmentAnim {
                    frames,
                    anim_ms: tex.anim_ms.max(1),
                    elapsed: 0.0,
                    current: 0,
                },
                TerrainFragment {
                    segment: pf.segment,
                    cell: IVec2::new(pf.x as i32, pf.y as i32),
                },
            ));
        } else {
            commands.spawn((
                Sprite {
                    image: img.clone(),
                    texture_atlas: Some(TextureAtlas {
                        layout: layout.clone(),
                        index: 0,
                    }),
                    custom_size: Some(Vec2::new(w_px, h_px)),
                    anchor: Anchor::TopLeft,
                    ..default()
                },
                Transform::from_xyz(pos.x, pos.y, 0.0),
                TerrainFragment {
                    segment: pf.segment,
                    cell: IVec2::new(pf.x as i32, pf.y as i32),
                },
            ));
        }
    }
    info!("Terrain spawned: {} fragments", gs.map.fragments.len());
    *done = true;
}

/// Tint each fragment by its anchor cell: black for unexplored land, dim for
/// remembered land, and normal color for currently visible terrain.
fn apply_fog_to_terrain(
    visibility: Res<VisibilityGrid>,
    mut terrain: Query<(&TerrainFragment, &mut Sprite)>,
) {
    for (fragment, mut sprite) in &mut terrain {
        sprite.color = if visibility.is_visible(fragment.cell) {
            Color::WHITE
        } else if visibility.is_explored(fragment.cell) {
            Color::srgb(0.42, 0.42, 0.48)
        } else {
            Color::srgb(0.025, 0.03, 0.045)
        };
    }
}

/// Global animation tick (DO animates all scheme instances in lockstep via
/// TGUI_ANIMATION::Update accumulating real time).
pub fn animate_fragments(time: Res<Time>, mut q: Query<&mut FragmentAnim>) {
    let dt = time.delta_secs() * 1000.0;
    for mut anim in q.iter_mut() {
        if anim.frames.len() <= 1 {
            continue;
        }
        anim.elapsed += dt;
        let frame_ms = anim.anim_ms as f32 / anim.frames.len() as f32;
        while anim.elapsed >= frame_ms {
            anim.elapsed -= frame_ms;
            anim.current = (anim.current + 1) % anim.frames.len();
        }
        // The atlas index lives on the Sprite; updated by the paired system.
    }
}

/// Apply computed frame index to the sprite atlas (kept separate so the query
/// above stays cheap and borrow-friendly).
pub fn apply_frame_indices(mut q: Query<(&FragmentAnim, &mut Sprite)>) {
    for (anim, mut sprite) in q.iter_mut() {
        if let Some(atlas) = sprite.texture_atlas.as_mut() {
            let idx = anim.frames[anim.current % anim.frames.len()];
            if atlas.index != idx {
                atlas.index = idx;
            }
        }
    }
}

/// Register the image loader plugin features we rely on (PNG decoding comes
/// from bevy's built-in ImageLoader via the `bevy_image` default features).
pub struct TerrainRenderPlugin;

impl Plugin for TerrainRenderPlugin {
    fn build(&self, app: &mut App) {
        // Ensure GroupAtlases resource exists before Update systems run.
        app.init_resource::<GroupAtlases>().add_systems(
            Update,
            (
                spawn_terrain,
                apply_fog_to_terrain,
                animate_fragments,
                apply_frame_indices.after(animate_fragments),
            )
                .chain(),
        );
    }
}
