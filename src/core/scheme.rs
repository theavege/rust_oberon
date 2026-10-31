//! Terrain scheme loader (.sch) — faithful port of `doschemes.cpp`.
//!
//! Ported behavior:
//! - `<Materials>` with `tg_id` (LoadSchMaterials, doschemes.cpp:385).
//! - `<Segments>/<Segment N>` with `<Terrain type T>` name/layer/difficulty.
//! - Layer→terrain-type hashtable + `GetTerrainType(user_id, TO_HIGH, sid)`
//!   remapping (doschemes.cpp:195–260): exact layer hit; above max → max;
//!   below 0 → 0; missing → scan upward then downward.
//! - `<Fragments>`: per-fragment `size` (width==height), `tg_id`, and a
//!   width×height `terrain_id` grid read **column-major** (`for i in w { for j
//!   in h }`, LoadSchFragmentTerrainId:420) and stored as `field[i][j]`.
//! - One implicit **default fragment** appended (`count + 1`, size 1×1, texture
//!   group index 0 = first group in the .dat, terrain id from
//!   `default_terrain_id`) — LoadSchFragments:478–505.

use std::collections::HashMap;

use crate::core::conf::{Reader, Tok};

pub const MAP_EMPTY_SURFACE: u8 = 255;

#[derive(Clone, Debug)]
pub struct Material {
    pub id: String,
    pub name: String,
    pub tg_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TerrainProps {
    pub user_type: usize,
    pub name: String,
    /// layer value doubles as the terrain id used by fragments (see
    /// `scheme.hashtable[layer] = terrain_index`).
    pub layer: u8,
    pub difficulty: f32,
}

#[derive(Clone, Debug)]
pub struct FragmentDef {
    pub width: usize,
    pub height: usize,
    /// Texture-group name from `tg_id` ("none" => None).
    pub tg_id: Option<String>,
    /// Remapped terrain ids, indexed [i][j] like the C `terrain_field[i][j]`.
    pub terrain_field: Vec<Vec<u8>>,
    /// Index into `Scheme::tex_groups` for the default fragment's group.
    pub is_default: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub name: String,
    pub terrains: Vec<TerrainProps>,
    pub fragments: Vec<FragmentDef>,
    /// layer -> terrain index (C `scheme.hashtable[segment][layer]`).
    hashtable: HashMap<u8, usize>,
    max_layer: u8,
}

impl Segment {
    /// Port of `GetTerrainType(user_terr_id, TO_HIGH, segment)`.
    pub fn get_terrain_type(&self, user_id: i64) -> u8 {
        if self.terrains.is_empty() {
            return 0;
        }
        if user_id > self.max_layer as i64 {
            return *self.hashtable.get(&self.max_layer).unwrap() as u8;
        }
        if user_id < 0 {
            return 0;
        }
        let uid = user_id as u8;
        if let Some(&idx) = self.hashtable.get(&uid) {
            return idx as u8;
        }
        // TO_HIGH: scan up, then down.
        let mut i = uid as i64;
        while i <= self.max_layer as i64 && !self.hashtable.contains_key(&(i as u8)) {
            i += 1;
        }
        if i <= self.max_layer as i64 {
            return *self.hashtable.get(&(i as u8)).unwrap() as u8;
        }
        let mut i = uid as i64;
        // Termination guard: when i reaches -1, `i.max(0)` would re-test key 0
        // forever if absent. The upward scan above already guarantees that any
        // reachable id has a hit at or below it (key 0 is always present in DO
        // schemes), so treat i < 0 as "no hit" and stop.
        while i >= 0 && !self.hashtable.contains_key(&(i as u8)) {
            i -= 1;
        }
        if i >= 0 {
            return *self.hashtable.get(&(i as u8)).unwrap() as u8;
        }
        0
    }

    /// Mirrors C `CreateHashTable` + the `qsort(..., compare)` performed in
    /// `LoadSchSegment` *before* it (doschemes.cpp:729): terrain props are
    /// sorted by layer ascending, then `hashtable[layer] = index`.
    fn finalize(&mut self) {
        // doschemes.cpp:176 — qsort comparator: el1->layer - el2->layer.
        // Rust's sort_by is stable; qsort isn't, but layers are unique in
        // practice so the orderings agree.
        self.terrains.sort_by_key(|t| t.layer);
        self.hashtable.clear();
        for (ti, t) in self.terrains.iter().enumerate() {
            self.hashtable.insert(t.layer, ti);
        }
        self.max_layer = self.terrains.last().map(|t| t.layer).unwrap_or(0);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Scheme {
    pub name: String,
    pub author: String,
    pub materials: Vec<Material>,
    pub segments: Vec<Segment>,
    /// Ordered texture-group names as they appear in the scheme .dat manifest
    /// (index == C `ReadTextureGroup` result).
    pub tex_groups: Vec<String>,
}

impl Scheme {
    /// Parse a .sch text. `group_names` are the texture groups present in the
    /// matching extracted manifest (in file order).
    pub fn parse(text: &str) -> Result<Self, String> {
        let toks = crate::core::conf::tokenize(text);
        let mut r = Reader::new(&toks);
        let mut sch = Scheme::default();

        while let Some(tok) = r.peek().cloned() {
            match tok {
                Tok::Word(w) => match w.as_str() {
                    "name" | "author" => {
                        // Only consume the pair when the value really follows;
                        // otherwise fall through to the generic +1 so we never
                        // swallow the next section's Open token.
                        let is_str = matches!(r.toks.get(r.pos + 1), Some(Tok::Str(_)));
                        if is_str {
                            r.pos += 1;
                            let v = r.str().unwrap_or_default();
                            if w.as_str() == "name" {
                                sch.name = v;
                            } else {
                                sch.author = v;
                            }
                        } else {
                            r.pos += 1;
                        }
                    }
                    _ => {
                        r.pos += 1;
                    }
                },
                Tok::Open(name) if name == "Materials" => {
                    // `read_kv_int` expects the cursor to be inside the section.
                    r.pos += 1;
                    let count = read_kv_int(&mut r, "count").unwrap_or(0) as usize;
                    for _ in 0..count {
                        expect_open(&mut r, |n| n.starts_with("Material "))?;
                        let mut m = Material {
                            id: String::new(),
                            name: String::new(),
                            tg_id: None,
                        };
                        loop {
                            match r.peek().cloned() {
                                Some(Tok::Word(k)) => match k.as_str() {
                                    "id" => {
                                        r.pos += 1;
                                        m.id = r.str().unwrap_or_default();
                                    }
                                    "name" => {
                                        r.pos += 1;
                                        m.name = r.str().unwrap_or_default();
                                    }
                                    "tg_id" => {
                                        r.pos += 1;
                                        m.tg_id = quoted_or_none(&mut r);
                                    }
                                    _ => {
                                        r.pos += 1;
                                    }
                                },
                                Some(Tok::Close(_)) => {
                                    r.pos += 1;
                                    break;
                                }
                                None => break,
                                _ => {
                                    r.pos += 1;
                                }
                            }
                        }
                        sch.materials.push(m);
                    }
                    // </Material N> closes are nested inside <Materials>, so a
                    // name-matched skip would never terminate; use depth-over-any-tag.
                    r.skip_section("");
                }
                Tok::Open(name) if name == "Segments" => {
                    loop {
                        match r.peek().cloned() {
                            Some(Tok::Open(n)) if n.starts_with("Segment ") => {
                                let mut seg = Segment::default();
                                r.pos += 1;
                                loop {
                                    match r.peek().cloned() {
                                        Some(Tok::Word(k)) if k == "name" => {
                                            if matches!(r.toks.get(r.pos + 1), Some(Tok::Str(_))) {
                                                r.pos += 1;
                                                seg.name = r.str().unwrap_or_default();
                                            } else {
                                                r.pos += 1;
                                            }
                                        }
                                        Some(Tok::Word(k)) if k == "count" => {
                                            r.pos += 1;
                                            let _ = r.int();
                                        }
                                        Some(Tok::Open(t)) if t.starts_with("Terrain type ") => {
                                            let utype: usize = t["Terrain type ".len()..]
                                                .trim()
                                                .parse()
                                                .unwrap_or(0);
                                            r.pos += 1;
                                            let mut tp = TerrainProps {
                                                user_type: utype,
                                                name: String::new(),
                                                layer: 0,
                                                difficulty: 0.0,
                                            };
                                            loop {
                                                match r.peek().cloned() {
                                                    Some(Tok::Word(k)) => match k.as_str() {
                                                        "name" => {
                                                            r.pos += 1;
                                                            tp.name = r.str().unwrap_or_default();
                                                        }
                                                        "layer" => {
                                                            r.pos += 1;
                                                            tp.layer = r.int().unwrap_or(0) as u8;
                                                        }
                                                        "difficulty" => {
                                                            r.pos += 1;
                                                            tp.difficulty = r
                                                                .word()
                                                                .and_then(|v| v.parse().ok())
                                                                .unwrap_or(0.0);
                                                        }
                                                        _ => {
                                                            r.pos += 1;
                                                        }
                                                    },
                                                    Some(Tok::Close(_)) => {
                                                        r.pos += 1;
                                                        break;
                                                    }
                                                    None => break,
                                                    _ => {
                                                        r.pos += 1;
                                                    }
                                                }
                                            }
                                            seg.terrains.push(tp);
                                        }
                                        Some(Tok::Open(f)) if f == "Fragments" => {
                                            // Fragment terrain ids are remapped as they load.
                                            seg.finalize();
                                            r.pos += 1;
                                            load_fragments(&mut r, &mut seg)?;
                                        }
                                        // Skip any other nested section (Layers,
                                        // Objects, ...) as a whole — otherwise its
                                        // `</Objects>` close would be mistaken for
                                        // the segment's own close tag.
                                        Some(Tok::Open(other)) => {
                                            r.pos += 1;
                                            skip_nested(&mut r, &other);
                                        }
                                        Some(Tok::Close(_)) => {
                                            // </Segment N> — leave it for the
                                            // consumer below (do NOT skip_nested:
                                            // that would already have eaten it).
                                            break;
                                        }
                                        _ => {
                                            r.pos += 1;
                                        }
                                    }
                                }
                                // The inner loop breaks on the segment's own Close
                                // token without consuming it (or at EOF). Consume
                                // only if it really is a Close.
                                match r.peek().cloned() {
                                    Some(Tok::Close(_)) => r.pos += 1,
                                    _ => return Err("unterminated <Segment>".into()),
                                }
                                seg.finalize();
                                sch.segments.push(seg);
                            }
                            Some(Tok::Close(n)) if n == "Segments" => {
                                r.pos += 1;
                                break;
                            }
                            _ => r.pos += 1,
                        }
                    }
                }
                Tok::Open(other) => {
                    r.pos += 1;
                    skip_nested(&mut r, &other);
                }
                _ => {
                    r.pos += 1;
                }
            }
        }
        Ok(sch)
    }
}

fn quoted_or_none(r: &mut Reader) -> Option<String> {
    match r.peek().cloned() {
        Some(Tok::Str(s)) => {
            r.pos += 1;
            Some(s)
        }
        Some(Tok::Word(w)) if w == "none" => {
            r.pos += 1;
            None
        }
        _ => None,
    }
}

fn read_kv_int(r: &mut Reader, key: &str) -> Option<i64> {
    // scan forward within current section until key found at top level
    for _ in 0..8 {
        match r.peek().cloned() {
            Some(Tok::Word(k)) if k == key => {
                r.pos += 1;
                return r.int();
            }
            Some(Tok::Word(_)) => {
                r.pos += 2; // skip unknown pair
            }
            _ => return None,
        }
    }
    None
}

fn expect_open<F: Fn(&str) -> bool>(r: &mut Reader, pred: F) -> Result<String, String> {
    match r.peek().cloned() {
        Some(Tok::Open(n)) if pred(&n) => {
            r.pos += 1;
            Ok(n)
        }
        other => Err(format!("expected open section, got {other:?}")),
    }
}

fn skip_nested(r: &mut Reader, name: &str) {
    let mut depth = 1;
    while r.pos < r.toks.len() && depth > 0 {
        match r.toks[r.pos].clone() {
            Tok::Open(n) if n == name => depth += 1,
            Tok::Close(n) if n == name => depth -= 1,
            _ => {}
        }
        r.pos += 1;
    }
}

/// Port of LoadSchFragments (doschemes.cpp:478).
///
/// NOTE: in plastic.sch the `tg_default_id` / `default_terrain_id` keys come
/// *before* `count` and the fragment boxes inside `<Fragments>`; the original
/// TCONF reader does positional reads, so we scan forward for those two keys
/// first (bounded by the section's own close tag), then parse fragments.
fn load_fragments(r: &mut Reader, seg: &mut Segment) -> Result<(), String> {
    // `r.pos` is just after the <Fragments> Open token; every Close token
    // counted below therefore belongs to a nested box or to </Fragments>.

    // --- header keys (position-independent scan within this section) ---
    let mut default_tid: i64 = 0;
    {
        let save = r.pos;
        let mut depth = 1usize;
        while r.pos < r.toks.len() && depth > 0 {
            if let Tok::Close(n) = &r.toks[r.pos] {
                if n == "Fragments" {
                    break; // end of this section — don't underflow depth
                }
            }
            match &r.toks[r.pos] {
                Tok::Open(_) => depth += 1,
                Tok::Close(_) => depth -= 1,
                Tok::Word(k) if k == "tg_default_id" => {
                    r.pos += 1;
                    let _ = quoted_or_none(r);
                    continue;
                }
                Tok::Word(k) if k == "default_terrain_id" => {
                    r.pos += 1;
                    default_tid = r.int().unwrap_or(0);
                }
                _ => {}
            }
            r.pos += 1;
        }
        r.pos = save;
    }

    // --- fragment boxes ---
    // `count` lives inside <Terrain fragments>, which sits *after* our scan
    // position (the header keys were consumed by the bounded scan above).
    // Continue scanning forward for it, staying within this <Fragments>
    // section's remaining tokens (no nested boxes precede it).
    let mut count: usize = 0;
    while r.pos < r.toks.len() {
        match r.peek().cloned() {
            Some(Tok::Word(k)) if k == "count" => {
                r.pos += 1;
                count = r.int().unwrap_or(0) as usize;
                break;
            }
            Some(Tok::Word(_)) => {
                r.pos += 1; // skip unknown scalar key
            }
            Some(Tok::Str(_)) => {
                r.pos += 1;
            }
            Some(Tok::Open(_)) | Some(Tok::Close(_)) => break, // fragments or section end
            None => break,
        }
    }
    for fid in 0..count {
        match r.peek().cloned() {
            Some(Tok::Open(n)) if n == format!("Fragment {fid}") => {
                r.pos += 1;
            }
            other => return Err(format!("expected <Fragment {fid}>, got {other:?}")),
        }
        let mut frag = FragmentDef {
            width: 1,
            height: 1,
            tg_id: None,
            terrain_field: Vec::new(),
            is_default: false,
        };
        loop {
            match r.peek().cloned() {
                Some(Tok::Word(k)) => match k.as_str() {
                    "size" => {
                        r.pos += 1;
                        let s = r.int().unwrap_or(1) as usize;
                        frag.width = s;
                        frag.height = s; // C: height = width
                    }
                    "tg_id" => {
                        r.pos += 1;
                        frag.tg_id = quoted_or_none(r);
                    }
                    "terrain_id" => {
                        r.pos += 1;
                        // column-major: for i in width { for j in height }
                        let mut field = vec![vec![0u8; frag.height]; frag.width];
                        for i in 0..frag.width {
                            for j in 0..frag.height {
                                let tid = r.int().unwrap_or(0);
                                field[i][j] = seg.get_terrain_type(tid);
                            }
                        }
                        frag.terrain_field = field;
                    }
                    _ => {
                        r.pos += 1;
                    }
                },
                Some(Tok::Close(_)) => break,
                None => break,
                Some(_) => break,
            }
        }
        match r.peek().cloned() {
            Some(Tok::Close(_)) => r.pos += 1, // </Fragment N>
            _ => return Err(format!("unterminated <Fragment {fid}>")),
        }
        seg.fragments.push(frag);
    }

    // implicit default fragment (count+1-th box)
    let def = FragmentDef {
        width: 1,
        height: 1,
        tg_id: None,
        terrain_field: vec![vec![seg.get_terrain_type(default_tid)]],
        is_default: true,
    };
    // tg_default_id: C resolves the group but always uses group index 0
    // (`SetTextures(scheme.tex_table.groups)` — doschemes.cpp:495).
    // `r` sits just after the last </Fragment N>; consume </Fragments>.
    match r.peek().cloned() {
        Some(Tok::Close(n)) if n == "Fragments" => r.pos += 1,
        _ => return Err("expected </Fragments>".into()),
    }
    seg.fragments.push(def);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port-parity check against vendored original data:
    /// plastic.sch segment 0 defines terrain types 0..6 with layers
    /// [5, 7, 8, 10, 30, 255, 0] (verified from assets/do/schemes/plastic.sch).
    fn load_plastic() -> Scheme {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/do/schemes");
        let text = std::fs::read_to_string(dir.join("plastic.sch")).expect("plastic.sch");
        Scheme::parse(&text).expect("parse plastic.sch")
    }

    #[test]
    fn plastic_segment0_terrains() {
        let sch = load_plastic();
        assert_eq!(sch.name, "Plastic World");
        // plastic.sch defines 3 segments (0 Underground, 1 …, 2 …).
        assert_eq!(sch.segments.len(), 3);
        let seg = &sch.segments[0];
        // finalize() sorts by layer ascending (C qsort, doschemes.cpp:729),
        // so the post-load order is 0, 5, 7, 8, 10, 30, 255.
        let layers: Vec<u8> = seg.terrains.iter().map(|t| t.layer).collect();
        assert_eq!(layers, vec![0, 5, 7, 8, 10, 30, 255]);
    }

    #[test]
    fn get_terrain_type_parity() {
        let sch = load_plastic();
        let seg = &sch.segments[0];
        // After the C qsort-by-layer, hashtable: layer -> terrain index is
        // 0->0, 5->1, 7->2, 8->3, 10->4, 30->5, 255->6.
        assert_eq!(seg.get_terrain_type(5), 1);
        assert_eq!(seg.get_terrain_type(10), 4);
        assert_eq!(seg.get_terrain_type(255), 6);
        assert_eq!(seg.get_terrain_type(0), 0);
        // missing ids scan upward (TO_HIGH):
        assert_eq!(seg.get_terrain_type(6), 2); // 6 -> up to 7 -> idx 2
        assert_eq!(seg.get_terrain_type(9), 4); // 9 -> up to 10 -> idx 4
        assert_eq!(seg.get_terrain_type(11), 5); // 11 -> up to 30 -> idx 5
        assert_eq!(seg.get_terrain_type(1), 1); // 1 -> up to 5 -> idx 1
                                                // out-of-range clamps:
        assert_eq!(seg.get_terrain_type(256), 6); // > max_layer 255 -> max entry
        assert_eq!(seg.get_terrain_type(-1), 0); // negative -> 0
    }

    #[test]
    fn fragment_field_is_column_major() {
        let sch = load_plastic();
        let seg = &sch.segments[0];
        // Fragment 1: size 5, terrain_id row-major in file: 20x 10 then 5x 255.
        // C reads column-major field[i][j], so last COLUMN (i=4) is all 255.
        let f1 = &seg.fragments[1];
        assert_eq!(f1.width, 5);
        assert_eq!(f1.height, 5);
        assert_eq!(f1.tg_id.as_deref(), Some("ug_grass_border_e"));
        for j in 0..5 {
            assert_eq!(f1.terrain_field[4][j], seg.get_terrain_type(255));
            assert_eq!(f1.terrain_field[0][j], seg.get_terrain_type(10));
        }
    }

    #[test]
    fn implicit_default_fragment_appended() {
        let sch = load_plastic();
        let seg = &sch.segments[0];
        // count fragments + 1 implicit default (LoadSchFragments: count+1 box).
        let last = seg.fragments.last().unwrap();
        assert!(last.is_default);
        assert_eq!(last.width, 1);
        assert_eq!(last.height, 1);
        // default_terrain_id 10 -> remapped via hashtable to index 4
        // (layer 10 is the 5th terrain after the C qsort-by-layer).
        assert_eq!(last.terrain_field[0][0], 4);
    }
}
// temp shim
