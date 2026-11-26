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
    ensure!(name.len() <= MAX_COMPONENT_BYTES, "file name is too long: {raw}");
    Ok(name)
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

/// `photo.jpg` becomes `photo (n).jpg`.
pub fn numbered(name: &str, n: usize) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => format!("{} ({n}){}", &name[..i], &name[i..]),
        _ => format!("{name} ({n})"),
    }
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
    fn case_collisions_get_numbered() {
        let paths = vec![vec!["dir".to_string(), "A.txt".to_string()], vec!["dir".to_string(), "a.txt".to_string()]];
        let out = unique_paths(paths).unwrap();
        assert_eq!(out[1], PathBuf::from("dir").join("a (1).txt"));
    }
}
