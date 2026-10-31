//! Reader for `manifest.json` files produced by `tools/doextract.py`.
//!
//! The manifests describe texture groups extracted from the original Dark
//! Oberon `.dat` archives (group order == C `TTEX_TABLE::Load` group order,
//! which is what `ReadTextureGroup` indices refer to — doschemes.cpp:782).
//!
//! This is a deliberately small JSON subset parser (no serde dependency):
//! it only understands the exact shape written by doextract.py:
//!   { "source": "...", "groups": [ { "name": "...", "textures": [
//!       { "id": "...", "frames_h": N, "frames_v": N, "anim_ms": N,
//!         "file": "..." } ] } ] }

#[derive(Clone, Debug, Default)]
pub struct TexEntry {
    pub id: String,
    pub frames_h: u32,
    pub frames_v: u32,
    pub anim_ms: u32,
    /// Path relative to the manifest directory.
    pub file: String,
}

#[derive(Clone, Debug)]
pub struct GroupEntry {
    pub name: String,
    pub textures: Vec<TexEntry>,
}

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub source: String,
    pub groups: Vec<GroupEntry>,
}

impl Manifest {
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut p = P {
            b: src.as_bytes(),
            i: 0,
        };
        p.skip_ws();
        p.expect(b'{')?;
        let mut m = Manifest::default();
        loop {
            p.skip_ws();
            if p.peek() == Some(b'}') {
                p.i += 1;
                break;
            }
            let key = p.string()?;
            p.skip_ws();
            p.expect(b':')?;
            p.skip_ws();
            match key.as_str() {
                "source" => m.source = p.string()?,
                "groups" => m.groups = p.array(|q| group(q))?,
                other => return Err(format!("unexpected key {other}")),
            }
            p.skip_ws();
            if p.peek() == Some(b',') {
                p.i += 1;
            }
        }
        Ok(m)
    }
}

fn group(p: &mut P) -> Result<GroupEntry, String> {
    p.expect(b'{')?;
    let mut g = GroupEntry {
        name: String::new(),
        textures: Vec::new(),
    };
    loop {
        p.skip_ws();
        if p.peek() == Some(b'}') {
            p.i += 1;
            break;
        }
        let key = p.string()?;
        p.skip_ws();
        p.expect(b':')?;
        p.skip_ws();
        match key.as_str() {
            "name" => g.name = p.string()?,
            "textures" => g.textures = p.array(tex)?,
            other => return Err(format!("unexpected group key {other}")),
        }
        p.skip_ws();
        if p.peek() == Some(b',') {
            p.i += 1;
        }
    }
    Ok(g)
}

fn tex(p: &mut P) -> Result<TexEntry, String> {
    p.expect(b'{')?;
    let mut t = TexEntry {
        frames_h: 1,
        frames_v: 1,
        ..TexEntry::default()
    };
    loop {
        p.skip_ws();
        if p.peek() == Some(b'}') {
            p.i += 1;
            break;
        }
        let key = p.string()?;
        p.skip_ws();
        p.expect(b':')?;
        p.skip_ws();
        match key.as_str() {
            "id" => t.id = p.string()?,
            "file" => t.file = p.string()?,
            "frames_h" => t.frames_h = p.number()? as u32,
            "frames_v" => t.frames_v = p.number()? as u32,
            "anim_ms" => t.anim_ms = p.number()? as u32,
            other => return Err(format!("unexpected texture key {other}")),
        }
        p.skip_ws();
        if p.peek() == Some(b',') {
            p.i += 1;
        }
    }
    Ok(t)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn skip_ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}' at byte {}", c as char, self.i))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.skip_ws();
        self.expect(b'"')?;
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b'"' {
            if self.b[self.i] == b'\\' {
                self.i += 1; // consume escape char
            }
            self.i += 1;
        }
        let raw = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|e| e.to_string())?
            .to_string();
        self.expect(b'"')?;
        Ok(raw.replace("\\/", "/").replace("\\\"", "\""))
    }
    fn number(&mut self) -> Result<f64, String> {
        let start = self.i;
        while self.i < self.b.len()
            && (self.b[self.i].is_ascii_digit()
                || matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E'))
        {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i])
            .map_err(|e| e.to_string())?
            .parse::<f64>()
            .map_err(|e| e.to_string())
    }
    fn array<T>(&mut self, f: impl Fn(&mut Self) -> Result<T, String>) -> Result<Vec<T>, String> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b']') {
                self.i += 1;
                return Ok(out);
            }
            out.push(f(self)?);
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.i += 1;
            }
        }
    }
}
