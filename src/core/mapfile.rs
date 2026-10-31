//! Map file loader (.map) — faithful port of `domap.cpp` map loading.
//!
//! Ported behavior (see TMAP_SEGMENT::LoadMapFragments / LoadMapFragment,
//! domap.cpp:551–668):
//! - Each segment has `<Fragments>` with `count` and entries
//!   `fragment_N <fid> <x> <y>` placing a scheme fragment at cell (x,y).
//! - Out-of-map fragments are skipped with a warning; overlapping cells are
//!   skipped with a warning (the overlap test in C checks w-1×h-1 cells —
//!   preserved verbatim as quirk #1).
//! - Surface terrain ids are written per cell from the fragment's
//!   `terrain_field[i][j]` (UpdateTerrainId, domap.cpp:349).
//! - Empty cells (MAP_EMPTY_SURFACE=255) get an implicit default-fragment
//!   instance placed column-major (`for x { for y }`, domap.cpp:610–630).

use crate::core::conf::{Reader, Tok};
use crate::core::scheme::{Scheme, MAP_EMPTY_SURFACE};

#[derive(Clone, Debug)]
pub struct PlacedFragment {
    /// Index into `Scheme::segments[segment].fragments`.
    pub fid: usize,
    pub x: u32,
    pub y: u32,
    pub segment: usize,
}

#[derive(Clone, Debug)]
pub struct StartPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug)]
pub struct MapFile {
    pub name: String,
    pub author: String,
    pub width: u32,
    pub height: u32,
    pub scheme_name: String,
    pub start_points: Vec<StartPoint>,
    pub fragments: Vec<PlacedFragment>,
    /// Per-segment surface terrain ids, row-major [y * w + x].
    pub surfaces: Vec<Vec<u8>>,
}

impl MapFile {
    pub fn parse(text: &str, scheme: &Scheme) -> Result<Self, String> {
        let toks = crate::core::conf::tokenize(text);
        let mut r = Reader::new(&toks);
        let mut map = MapFile {
            name: String::new(),
            author: String::new(),
            width: 0,
            height: 0,
            scheme_name: String::new(),
            start_points: Vec::new(),
            fragments: Vec::new(),
            surfaces: Vec::new(),
        };

        // Top-level scalars until first section.
        while let Some(tok) = r.peek().cloned() {
            match tok {
                Tok::Word(w) => match w.as_str() {
                    "name" => {
                        r.pos += 1;
                        map.name = r.str().unwrap_or_default();
                    }
                    "author" => {
                        r.pos += 1;
                        map.author = r.str().unwrap_or_default();
                    }
                    "width" => {
                        r.pos += 1;
                        map.width = r.int().unwrap_or(0) as u32;
                    }
                    "height" => {
                        r.pos += 1;
                        map.height = r.int().unwrap_or(0) as u32;
                    }
                    "scheme" => {
                        r.pos += 1;
                        map.scheme_name = r.str().unwrap_or_default();
                    }
                    _ => {
                        r.pos += 1;
                    }
                },
                Tok::Open(n) if n == "Players" => {
                    r.pos += 1;
                    collect_start_points(&mut r, &mut map.start_points);
                }
                Tok::Open(n) if n.starts_with("Segment ") => {
                    let sid: usize = n["Segment ".len()..].trim().parse().unwrap_or(0);
                    r.pos += 1;
                    load_segment(&mut r, &mut map, scheme, sid)?;
                }
                Tok::Open(other) => {
                    r.pos += 1;
                    let mut depth = 1;
                    while r.pos < r.toks.len() && depth > 0 {
                        match r.toks[r.pos].clone() {
                            Tok::Open(n) if n == other => depth += 1,
                            Tok::Close(n) if n == other => depth -= 1,
                            _ => {}
                        }
                        r.pos += 1;
                    }
                }
                _ => r.pos += 1,
            }
        }
        Ok(map)
    }

    pub fn surface(&self, segment: usize, x: u32, y: u32) -> Option<u8> {
        self.surfaces
            .get(segment)?
            .get((y * self.width + x) as usize)
            .copied()
    }
}

fn collect_start_points(r: &mut Reader, out: &mut Vec<StartPoint>) {
    // scan tokens inside Players until its matching Close; accept any
    // `start_point_N x y` triple wherever it appears.
    let mut depth = 1usize;
    while r.pos < r.toks.len() && depth > 0 {
        match r.toks[r.pos].clone() {
            Tok::Open(_) => depth += 1,
            Tok::Close(_) => depth -= 1,
            Tok::Word(w) if w.starts_with("start_point_") => {
                r.pos += 1;
                if let (Some(x), Some(y)) = (r.int(), r.int()) {
                    out.push(StartPoint {
                        x: x as u32,
                        y: y as u32,
                    });
                }
                continue;
            }
            _ => {}
        }
        r.pos += 1;
    }
}

fn load_segment(
    r: &mut Reader,
    map: &mut MapFile,
    scheme: &Scheme,
    sid: usize,
) -> Result<(), String> {
    // ensure surface exists for this segment id (segments are dense 0..2 in DO)
    while map.surfaces.len() <= sid {
        map.surfaces
            .push(vec![MAP_EMPTY_SURFACE; (map.width * map.height) as usize]);
    }
    // Work on a local copy of the surface so `load_fragments` can take both
    // the map (for fragment bookkeeping) and the surface without aliasing.
    let mut surface = std::mem::take(&mut map.surfaces[sid]);

    loop {
        match r.peek().cloned() {
            Some(Tok::Open(n)) if n == "Fragments" => {
                r.pos += 1;
                load_fragments(r, map, scheme, sid, &mut surface)?;
            }
            Some(Tok::Open(other)) => {
                r.pos += 1;
                let mut depth = 1;
                while r.pos < r.toks.len() && depth > 0 {
                    match r.toks[r.pos].clone() {
                        Tok::Open(n) if n == other => depth += 1,
                        Tok::Close(n) if n == other => depth -= 1,
                        _ => {}
                    }
                    r.pos += 1;
                }
            }
            Some(Tok::Close(_)) => {
                r.pos += 1;
                break;
            }
            None => return Err(format!("unterminated <Segment {sid}>")),
            _ => {
                r.pos += 1;
            }
        }
    }

    // Fill empty cells with the implicit default fragment (index = last in
    // scheme segment list), scanning column-major like C (`for i in width`).
    let default_fid = scheme
        .segments
        .get(sid)
        .map(|s| s.fragments.len() - 1)
        .unwrap_or(0);
    let mut k = 0u32;
    let mut l = 0u32;
    loop {
        let mut stop = false;
        while k < map.width && !stop {
            if l >= map.height {
                l = 0;
            }
            while l < map.height && !stop {
                if surface[(l * map.width + k) as usize] == MAP_EMPTY_SURFACE {
                    stop = true;
                }
                if !stop {
                    l += 1;
                }
            }
            if !stop {
                k += 1;
            }
        }
        if !stop {
            break;
        }
        // C: NEW TTERR_FRAG(default, --k, --l, sid) then UpdateTerrainId
        let (cx, cy) = (k - 1, l - 1);
        map.fragments.push(PlacedFragment {
            fid: default_fid,
            x: cx,
            y: cy,
            segment: sid,
        });
        if let Some(frag) = scheme
            .segments
            .get(sid)
            .and_then(|s| s.fragments.get(default_fid))
        {
            for i in 0..frag.width {
                for j in 0..frag.height {
                    let gx = cx + i as u32;
                    let gy = cy + j as u32;
                    if gx < map.width && gy < map.height {
                        surface[(gy * map.width + gx) as usize] = frag.terrain_field[i][j];
                    }
                }
            }
        }
        k = cx + 1;
        l = cy + 1;
    }
    map.surfaces[sid] = surface;
    Ok(())
}

fn load_fragments(
    r: &mut Reader,
    map: &mut MapFile,
    scheme: &Scheme,
    sid: usize,
    surface: &mut [u8],
) -> Result<(), String> {
    loop {
        match r.peek().cloned() {
            Some(Tok::Word(k)) if k == "count" => {
                r.pos += 1;
                r.pos += 1; // count value not needed; entries are named
            }
            Some(Tok::Word(k)) if k.starts_with("fragment_") => {
                r.pos += 1;
                let fid = r.int().unwrap_or(0) as usize;
                let x = r.int().unwrap_or(0) as u32;
                let y = r.int().unwrap_or(0) as u32;

                let frag = scheme
                    .segments
                    .get(sid)
                    .and_then(|s| s.fragments.get(fid))
                    .ok_or(format!("unknown fragment id {fid} in segment {sid}"))?;

                let in_map = x < map.width
                    && y < map.height
                    && (x + frag.width as u32).saturating_sub(1) < map.width
                    && (y + frag.height as u32).saturating_sub(1) < map.height;
                if !in_map {
                    eprintln!("Warning: map fragment {fid} located out of map in segment {sid}.");
                    continue;
                }

                // Overlap test — verbatim port of the C quirk: loops run to
                // x+width-1 / y+height-1 (one cell short of full footprint).
                let mut overlaps = false;
                for i in x..x + frag.width as u32 - 1 {
                    for j in y..y + frag.height as u32 - 1 {
                        if surface[(j * map.width + i) as usize] != MAP_EMPTY_SURFACE {
                            eprintln!(
                                "Warning: Map fragment located on another fragment in segment {sid}."
                            );
                            overlaps = true;
                        }
                    }
                }
                if overlaps {
                    continue;
                }

                map.fragments.push(PlacedFragment {
                    fid,
                    x,
                    y,
                    segment: sid,
                });
                for i in 0..frag.width {
                    for j in 0..frag.height {
                        let gx = x + i as u32;
                        let gy = y + j as u32;
                        if gx < map.width && gy < map.height {
                            surface[(gy * map.width + gx) as usize] = frag.terrain_field[i][j];
                        }
                    }
                }
            }
            Some(Tok::Close(_)) => {
                r.pos += 1;
                return Ok(());
            }
            None => return Err(format!("unterminated <Fragments> in segment {sid}")),
            _ => {
                r.pos += 1;
            }
        }
    }
}
