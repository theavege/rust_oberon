//! Offline validation for the project's imported maps, scheme and texture manifest.

use std::collections::HashMap;
use std::path::{Component, Path};

use crate::core::{
    datamanifest::Manifest,
    mapfile::MapFile,
    scheme::{Scheme, MAP_EMPTY_SURFACE},
};

const MAX_MAP_DIMENSION: u32 = 1024;

pub fn validate_content(root: &Path) -> Result<usize, Vec<String>> {
    let mut errors = Vec::new();
    let manifest_path = root.join("scheme_plastic/manifest.json");
    let scheme_dir = root.join("schemes");
    let maps_dir = root.join("maps");
    let manifest_text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => Some(text),
        Err(error) => {
            errors.push(format!("{}:1: {error}", manifest_path.display()));
            None
        }
    };
    let manifest = manifest_text
        .as_deref()
        .and_then(|text| match Manifest::parse(text) {
            Ok(manifest) => Some(manifest),
            Err(error) => {
                errors.push(diagnostic(&manifest_path, text, &error));
                None
            }
        });

    let mut schemes = HashMap::<String, Scheme>::new();
    let mut validated = 0usize;
    match std::fs::read_dir(&scheme_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "sch") {
                    continue;
                }
                match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let before_source_validation = errors.len();
                        validate_scheme_source(&path, &text, &mut errors);
                        if errors.len() != before_source_validation {
                            continue;
                        }
                        match Scheme::parse(&text) {
                            Ok(scheme) => {
                                let before = errors.len();
                                validate_scheme(
                                    &path,
                                    &text,
                                    &scheme,
                                    manifest.as_ref(),
                                    &mut errors,
                                );
                                if errors.len() == before {
                                    validated += 1;
                                    schemes.insert(
                                        path.file_stem()
                                            .and_then(|stem| stem.to_str())
                                            .unwrap_or_default()
                                            .to_owned(),
                                        scheme,
                                    );
                                }
                            }
                            Err(error) => errors.push(diagnostic(&path, &text, &error)),
                        }
                    }
                    Err(error) => errors.push(format!("{}:1: {error}", path.display())),
                }
            }
        }
        Err(error) => errors.push(format!("{}:1: {error}", scheme_dir.display())),
    }

    match std::fs::read_dir(&maps_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "map") {
                    continue;
                }
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) => {
                        errors.push(format!("{}:1: {error}", path.display()));
                        continue;
                    }
                };
                let Some((width, height)) = validate_map_header(&path, &text, &mut errors) else {
                    continue;
                };
                let scheme_name = header_string(&text, "scheme").unwrap_or_default();
                let Some(scheme) = schemes.get(&scheme_name) else {
                    errors.push(diagnostic(
                        &path,
                        &text,
                        &format!("unknown or invalid scheme '{scheme_name}'"),
                    ));
                    continue;
                };
                let before = errors.len();
                validate_fragment_entries(&path, &text, scheme, width, height, &mut errors);
                match MapFile::parse(&text, scheme) {
                    Ok(map) => validate_map(&path, &text, &map, scheme, width, height, &mut errors),
                    Err(error) => errors.push(diagnostic(&path, &text, &error)),
                }
                if errors.len() == before {
                    validated += 1;
                }
            }
        }
        Err(error) => errors.push(format!("{}:1: {error}", maps_dir.display())),
    }

    if let Some(manifest) = &manifest {
        validate_manifest(
            root,
            &manifest_path,
            manifest_text.as_deref().unwrap_or_default(),
            manifest,
            &mut errors,
        );
    }
    if errors.is_empty() {
        Ok(validated)
    } else {
        Err(errors)
    }
}

fn validate_scheme_source(path: &Path, text: &str, errors: &mut Vec<String>) {
    for (index, line) in text.lines().enumerate() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("size") {
            continue;
        }
        let Some(size) = fields.next().and_then(|value| value.parse::<usize>().ok()) else {
            errors.push(format!(
                "{}:{}: fragment size must be an integer",
                path.display(),
                index + 1
            ));
            continue;
        };
        if !(1..=16).contains(&size) {
            errors.push(format!(
                "{}:{}: fragment size must be between 1 and 16",
                path.display(),
                index + 1
            ));
        }
    }
}

fn validate_fragment_entries(
    path: &Path,
    text: &str,
    scheme: &Scheme,
    map_width: u32,
    map_height: u32,
    errors: &mut Vec<String>,
) {
    let mut segment_id = None;
    for (line_index, line) in text.lines().enumerate() {
        let line_number = line_index + 1;
        let trimmed = line.trim();
        if let Some(name) = trimmed
            .strip_prefix("<Segment ")
            .and_then(|value| value.strip_suffix('>'))
        {
            segment_id = name.trim().parse::<usize>().ok();
            if segment_id.is_none() {
                errors.push(format!(
                    "{}:{line_number}: malformed segment index",
                    path.display()
                ));
            }
            continue;
        }
        if trimmed.starts_with("</Segment ") {
            segment_id = None;
            continue;
        }
        let mut fields = trimmed.split_whitespace();
        let Some(key) = fields.next().filter(|key| key.starts_with("fragment_")) else {
            continue;
        };
        let mut parse = || {
            Some((
                fields.next()?.parse::<usize>().ok()?,
                fields.next()?.parse::<u32>().ok()?,
                fields.next()?.parse::<u32>().ok()?,
            ))
        };
        let Some((fid, x, y)) = parse() else {
            errors.push(format!(
                "{}:{line_number}: malformed {key} entry",
                path.display()
            ));
            continue;
        };
        let Some(sid) = segment_id else {
            errors.push(format!(
                "{}:{line_number}: fragment entry is outside a segment",
                path.display()
            ));
            continue;
        };
        let Some(fragment) = scheme
            .segments
            .get(sid)
            .and_then(|segment| segment.fragments.get(fid))
        else {
            errors.push(format!(
                "{}:{line_number}: unknown fragment id {fid} in segment {sid}",
                path.display()
            ));
            continue;
        };
        if x as u64 + fragment.width as u64 > map_width as u64
            || y as u64 + fragment.height as u64 > map_height as u64
        {
            errors.push(format!(
                "{}:{line_number}: fragment {fid} extends beyond map bounds",
                path.display()
            ));
        }
    }
}

fn validate_scheme(
    path: &Path,
    text: &str,
    scheme: &Scheme,
    manifest: Option<&Manifest>,
    errors: &mut Vec<String>,
) {
    if scheme.name.trim().is_empty() {
        errors.push(diagnostic(path, text, "scheme has no name"));
    }
    if scheme.segments.is_empty() {
        errors.push(diagnostic(path, text, "scheme has no segments"));
    }
    for (sid, segment) in scheme.segments.iter().enumerate() {
        if segment.terrains.is_empty() {
            errors.push(diagnostic(
                path,
                text,
                &format!("segment {sid} has no terrain definitions"),
            ));
        }
        if segment.fragments.is_empty() {
            errors.push(diagnostic(
                path,
                text,
                &format!("segment {sid} has no fragments"),
            ));
        }
        for fragment in &segment.fragments {
            if fragment.width == 0
                || fragment.height == 0
                || fragment.terrain_field.len() != fragment.width
                || fragment
                    .terrain_field
                    .iter()
                    .any(|column| column.len() != fragment.height)
            {
                errors.push(diagnostic(
                    path,
                    text,
                    &format!("segment {sid} has a fragment with invalid dimensions"),
                ));
                break;
            }
        }
        if let Some(manifest) = manifest {
            for material in &scheme.materials {
                if let Some(group) = &material.tg_id {
                    if !manifest.groups.iter().any(|entry| entry.name == *group) {
                        errors.push(diagnostic(
                            path,
                            text,
                            &format!(
                                "material '{}' references missing texture group '{group}'",
                                material.name
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn validate_manifest(
    root: &Path,
    path: &Path,
    source: &str,
    manifest: &Manifest,
    errors: &mut Vec<String>,
) {
    if manifest.groups.is_empty() {
        errors.push(format!(
            "{}:1: manifest has no texture groups",
            path.display()
        ));
    }
    for group in &manifest.groups {
        if group.name.is_empty() || group.textures.is_empty() {
            errors.push(format!(
                "{}:{}: texture group '{}' is empty",
                path.display(),
                source_line(source, &group.name),
                group.name
            ));
        }
        for texture in &group.textures {
            let relative = Path::new(&texture.file);
            if texture.frames_h == 0 || texture.frames_v == 0 {
                errors.push(format!(
                    "{}:{}: texture '{}' has zero frames",
                    path.display(),
                    source_line(source, &texture.file),
                    texture.id
                ));
                continue;
            }
            if relative.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                errors.push(format!(
                    "{}:{}: texture path '{}' escapes the asset directory",
                    path.display(),
                    source_line(source, &texture.file),
                    texture.file
                ));
                continue;
            }
            if !root.join("scheme_plastic").join(relative).is_file() {
                errors.push(format!(
                    "{}:{}: texture file '{}' is missing",
                    path.display(),
                    source_line(source, &texture.file),
                    texture.file
                ));
            }
        }
    }
}

fn source_line(source: &str, value: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains(value))
        .map_or(1, |index| index + 1)
}

fn validate_map_header(path: &Path, text: &str, errors: &mut Vec<String>) -> Option<(u32, u32)> {
    let width = header_integer(text, "width");
    let height = header_integer(text, "height");
    let Some((width, width_line)) = width else {
        errors.push(diagnostic(path, text, "map is missing a valid width"));
        return None;
    };
    let Some((height, height_line)) = height else {
        errors.push(diagnostic(path, text, "map is missing a valid height"));
        return None;
    };
    if width == 0 || width > MAX_MAP_DIMENSION {
        errors.push(format!(
            "{}:{width_line}: width must be between 1 and {MAX_MAP_DIMENSION}",
            path.display()
        ));
        return None;
    }
    if height == 0 || height > MAX_MAP_DIMENSION {
        errors.push(format!(
            "{}:{height_line}: height must be between 1 and {MAX_MAP_DIMENSION}",
            path.display()
        ));
        return None;
    }
    if header_string(text, "name").is_none_or(|name| name.trim().is_empty()) {
        errors.push(diagnostic(path, text, "map is missing a name"));
        return None;
    }
    Some((width, height))
}

fn validate_map(
    path: &Path,
    text: &str,
    map: &MapFile,
    scheme: &Scheme,
    width: u32,
    height: u32,
    errors: &mut Vec<String>,
) {
    if map.width != width || map.height != height {
        errors.push(diagnostic(
            path,
            text,
            "parsed map dimensions do not match the header",
        ));
    }
    if map.scheme_name != scheme_name_from_text(text).unwrap_or_default() {
        errors.push(diagnostic(
            path,
            text,
            "map scheme name does not match the loaded scheme",
        ));
    }
    if map.start_points.is_empty() {
        errors.push(diagnostic(path, text, "map has no player start points"));
    }
    for (index, point) in map.start_points.iter().enumerate() {
        if point.x >= width || point.y >= height {
            let line = text
                .lines()
                .position(|line| line.contains(&format!("start_point_{index}")))
                .map_or(1, |line| line + 1);
            errors.push(format!(
                "{}:{line}: player start point {index} is outside the map",
                path.display()
            ));
        }
    }
    for (sid, surface) in map.surfaces.iter().enumerate() {
        if surface.len() != (width * height) as usize {
            errors.push(diagnostic(
                path,
                text,
                &format!("segment {sid} has an invalid surface size"),
            ));
            continue;
        }
        if let Some(segment) = scheme.segments.get(sid) {
            for (cell, terrain_id) in surface.iter().enumerate() {
                if *terrain_id != MAP_EMPTY_SURFACE
                    && *terrain_id as usize >= segment.terrains.len()
                {
                    errors.push(diagnostic(
                        path,
                        text,
                        &format!("segment {sid} has an invalid terrain id at cell {cell}"),
                    ));
                    break;
                }
            }
        }
    }
    for fragment in &map.fragments {
        let Some(definition) = scheme
            .segments
            .get(fragment.segment)
            .and_then(|segment| segment.fragments.get(fragment.fid))
        else {
            errors.push(diagnostic(
                path,
                text,
                "map references a missing fragment definition",
            ));
            continue;
        };
        if fragment.x + definition.width as u32 > width
            || fragment.y + definition.height as u32 > height
        {
            errors.push(diagnostic(path, text, "fragment extends beyond map bounds"));
        }
    }
}

fn header_integer(text: &str, key: &str) -> Option<(u32, usize)> {
    text.lines().enumerate().find_map(|(index, line)| {
        let mut fields = line.split_whitespace();
        if fields.next()? != key {
            return None;
        }
        Some((fields.next()?.parse::<u32>().ok()?, index + 1))
    })
}

fn header_string(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != key {
            return None;
        }
        let value = line.split_once('"')?.1.split_once('"')?.0;
        Some(value.to_owned())
    })
}

fn scheme_name_from_text(text: &str) -> Option<String> {
    header_string(text, "scheme")
}

fn diagnostic(path: &Path, source: &str, error: &str) -> String {
    format!("{}:{}: {error}", path.display(), error_line(source, error))
}

fn error_line(source: &str, error: &str) -> usize {
    if let Some(index) = error.find("byte ") {
        if let Some(byte) = error[index + 5..]
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<usize>().ok())
        {
            return source.as_bytes()[..byte.min(source.len())]
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1;
        }
    }
    if let Some(index) = error.find("unknown fragment id ") {
        if let Some(fid) = error[index + "unknown fragment id ".len()..]
            .split_whitespace()
            .next()
        {
            if let Some(line) = source.lines().position(|line| {
                let mut fields = line.split_whitespace();
                fields
                    .next()
                    .is_some_and(|key| key.starts_with("fragment_"))
                    && fields.next() == Some(fid)
            }) {
                return line + 1;
            }
        }
    }
    if let Some(index) = error.find("<Fragment ") {
        if let Some(section) = error[index..].split('>').next() {
            if let Some(line) = source.lines().position(|line| line.contains(section)) {
                return line + 1;
            }
        }
    }
    for key in ["width", "height", "name", "author", "scheme"] {
        if error.contains(key) {
            if let Some(line) = source
                .lines()
                .position(|line| line.split_whitespace().next() == Some(key))
            {
                return line + 1;
            }
        }
    }
    if error.contains("start point") {
        if let Some(line) = source.lines().position(|line| line.contains("<Players>")) {
            return line + 1;
        }
    }
    source.lines().count().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scheme::{FragmentDef, Segment};
    use std::path::PathBuf;

    #[test]
    fn parser_diagnostics_point_to_the_failing_line() {
        let source = "width 10\nfragment_0 99 2 3\n";
        assert_eq!(error_line(source, "unknown fragment id 99 in segment 0"), 2);
        assert_eq!(error_line("{\n  \"x\" 1\n}", "expected ':' at byte 7"), 2);
    }

    #[test]
    fn repository_content_validates() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/do");
        assert!(validate_content(&root).is_ok());
    }

    #[test]
    fn out_of_bounds_fragment_reports_its_source_line() {
        let mut scheme = Scheme::default();
        let mut segment = Segment::default();
        segment.fragments.push(FragmentDef {
            width: 2,
            height: 2,
            tg_id: None,
            terrain_field: vec![vec![0; 2]; 2],
            is_default: false,
        });
        scheme.segments.push(segment);
        let text = "<Segment 0>\n<Fragments>\nfragment_0 0 3 3\n</Fragments>\n</Segment 0>";
        let mut errors = Vec::new();

        validate_fragment_entries(Path::new("broken.map"), text, &scheme, 4, 4, &mut errors);

        assert!(errors.iter().any(|error| error.contains("broken.map:3:")));
    }
}
