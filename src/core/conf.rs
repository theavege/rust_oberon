//! Tokenizer for Dark Oberon conf files (.map / .sch / .rac).
//! Faithful port of the token stream produced by `TCONF_FILE::GetToken`
//! (dofile.cpp): `#` starts a line comment, `"..."` is a quoted string,
//! `<Name>` / `</Name>` are section open/close tokens, everything else
//! separated by whitespace is a plain word.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tok {
    /// Plain word or integer token.
    Word(String),
    /// Quoted string.
    Str(String),
    /// `<Section>`
    Open(String),
    /// `</Section>`
    Close(String),
}

pub fn tokenize(src: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let b: Vec<char> = src.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == '#' {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            let start = i;
            while i < b.len() && b[i] != '"' {
                i += 1;
            }
            out.push(Tok::Str(b[start..i].iter().collect()));
            i += 1; // closing quote
            continue;
        }
        if c == '<' {
            // `<Name>` or `</Name>` — note terrain_id lines never contain '<'.
            let start = i;
            while i < b.len() && b[i] != '>' {
                i += 1;
            }
            if i < b.len() {
                let raw: String = b[start + 1..i].iter().collect();
                i += 1;
                if let Some(name) = raw.strip_prefix('/') {
                    out.push(Tok::Close(name.trim().to_string()));
                } else {
                    out.push(Tok::Open(raw.trim().to_string()));
                }
            }
            continue;
        }
        // plain word
        let start = i;
        while i < b.len() && !b[i].is_whitespace() && b[i] != '#' && b[i] != '<' && b[i] != '"' {
            i += 1;
        }
        out.push(Tok::Word(b[start..i].iter().collect()));
    }
    out
}

/// Cursor over tokens with helpers mirroring TCONF_FILE reads.
pub struct Reader<'a> {
    pub toks: &'a [Tok],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(toks: &'a [Tok]) -> Self {
        Self { toks, pos: 0 }
    }

    pub fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    pub fn word(&mut self) -> Option<String> {
        match self.toks.get(self.pos) {
            Some(Tok::Word(w)) => {
                self.pos += 1;
                Some(w.clone())
            }
            _ => None,
        }
    }

    pub fn int(&mut self) -> Option<i64> {
        let w = self.word()?;
        w.parse().ok()
    }

    pub fn str(&mut self) -> Option<String> {
        match self.toks.get(self.pos) {
            Some(Tok::Str(s)) => {
                self.pos += 1;
                Some(s.clone())
            }
            _ => None,
        }
    }

    /// Consume tokens until the section we are inside of closes (or EOF).
    /// Depth is tracked over *any* Open/Close token pair, so it terminates even
    /// when nested boxes use different tag names than the enclosing section
    /// (e.g. `</Material N>` inside `<Materials>`). Passing a specific `name`
    /// restricts counting to that tag only (used for same-name nesting).
    pub fn skip_section(&mut self, name: &str) {
        let mut depth = 1i32;
        while self.pos < self.toks.len() && depth > 0 {
            match &self.toks[self.pos] {
                Tok::Open(n) if name.is_empty() || n == name => depth += 1,
                Tok::Close(n) if name.is_empty() || n == name => depth -= 1,
                _ => {}
            }
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_basic() {
        let toks = tokenize("width 80 # comment\nname \"trial map\"\n<Players>\nx 1\n</Players>");
        assert_eq!(
            toks,
            vec![
                Tok::Word("width".into()),
                Tok::Word("80".into()),
                Tok::Word("name".into()),
                Tok::Str("trial map".into()),
                Tok::Open("Players".into()),
                Tok::Word("x".into()),
                Tok::Word("1".into()),
                Tok::Close("Players".into()),
            ]
        );
    }

    #[test]
    fn reader_int_and_str() {
        let toks = tokenize("count 5 id \"abc\"");
        let mut r = Reader::new(&toks);
        assert_eq!(r.word().as_deref(), Some("count"));
        assert_eq!(r.int(), Some(5));
        assert_eq!(r.word().as_deref(), Some("id"));
        assert_eq!(r.str().as_deref(), Some("abc"));
    }
}
