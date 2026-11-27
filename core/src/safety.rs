//! Turns file paths from a peer into paths that are safe to create on this device.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{bail, ensure, Result};

const MAX_PATH_BYTES: usize = 4096;
const MAX_COMPONENT_BYTES: usize = 255;

/// Splits a `/`-separated relative path into cleaned components. Rejects anything
/// that could point outside the destination folder.
pub fn clean_relative_path(path: &str) -> Result<Vec<String>> {
    ensure!(!path.is_empty() && path.len() <= MAX_PATH_BYTES, "invalid file path");
    let mut parts = Vec::new();
    for raw in path.split('/') {
        ensure!(!raw.is_empty() && raw != "." && raw != "..", "unsafe file path: {path}");
        parts.push(clean_component(raw)?);
    }
    Ok(parts)
}

/// Replaces characters Windows can't store (including `\` and `:`, which could
/// otherwise change the meaning of the path) and defuses reserved device names.
fn clean_component(raw: &str) -> Result<String> {
    let mut name: String = raw
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }
    if name.is_empty() {
        name.push('_');
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if is_reserved(&stem) {
        name.insert(0, '_');
    }
    // Too long to store: shortened, keeping the extension, rather than refused.
    Ok(fit(&name, ""))
}

/// `name` followed by `tail` before the extension, cut down so the whole is at most
/// 255 bytes. Cuts the stem, never the extension, and never in the middle of a
/// character.
fn fit(name: &str, tail: &str) -> String {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let room = MAX_COMPONENT_BYTES.saturating_sub(tail.len() + ext.len());
    let mut end = stem.len().min(room);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{tail}{ext}", &stem[..end])
}

fn is_reserved(stem: &str) -> bool {
    matches!(stem, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Makes paths unique ignoring case, so two offered names (`A.txt`, `a.txt`) can
/// never end up writing the same file.
pub fn unique_paths(paths: Vec<Vec<String>>) -> Result<Vec<PathBuf>> {
    let mut files = HashSet::new();
    let mut dirs = HashSet::new();
    let mut out = Vec::with_capacity(paths.len());
    for parts in paths {
        for depth in 1..parts.len() {
            if files.contains(&key(&parts[..depth])) {
                bail!("a file and a folder share the name {}", parts[..depth].join("/"));
            }
        }
        let mut candidate = parts.clone();
        let mut n = 1;
        while files.contains(&key(&candidate)) || dirs.contains(&key(&candidate)) {
            let last = candidate.len() - 1;
            candidate[last] = numbered(&parts[last], n);
            n += 1;
        }
        for depth in 1..candidate.len() {
            dirs.insert(key(&candidate[..depth]));
        }
        files.insert(key(&candidate));
        out.push(candidate.iter().collect());
    }
    Ok(out)
}

/// `photo.jpg` becomes `photo (n).jpg`, shortened if that would be too long a name.
pub fn numbered(name: &str, n: usize) -> String {
    fit(name, &format!(" ({n})"))
}

fn key(parts: &[String]) -> String {
    parts.join("/").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_is_rejected() {
        for bad in ["../x", "a/../../x", "/etc/passwd", "a//b", "./a", ""] {
            assert!(clean_relative_path(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn dangerous_characters_are_replaced() {
        assert_eq!(clean_relative_path("C:\\Windows\\x").unwrap(), vec!["C__Windows_x"]);
        assert_eq!(clean_relative_path("a/CON.txt").unwrap(), vec!["a", "_CON.txt"]);
        assert_eq!(clean_relative_path("a/name. ").unwrap(), vec!["a", "name"]);
        assert_eq!(clean_relative_path("a/...").unwrap(), vec!["a", "_"]);
    }

    #[test]
    fn long_names_are_shortened_not_refused() {
        let long = format!("{}.jpg", "é".repeat(200));
        let cleaned = clean_relative_path(&long).unwrap();
        assert!(cleaned[0].len() <= 255 && cleaned[0].ends_with(".jpg"));
        // Numbering and the reserved-name prefix can't push a name past the limit either.
        let exact = format!("{}.mp4", "a".repeat(250));
        assert!(numbered(&exact, 12).len() <= 255 && numbered(&exact, 12).ends_with(" (12).mp4"));
        assert!(clean_relative_path(&format!("CON.{}", "b".repeat(251))).unwrap()[0].len() <= 255);
    }

    #[test]
    fn case_collisions_get_numbered() {
        let paths = vec![vec!["dir".to_string(), "A.txt".to_string()], vec!["dir".to_string(), "a.txt".to_string()]];
        let out = unique_paths(paths).unwrap();
        assert_eq!(out[1], PathBuf::from("dir").join("a (1).txt"));
    }
}
