//! Freedesktop-style bookmarks (`~/.config/gtk-3.0/bookmarks`, lines
//! `uri name...`). Same file as Nautilus: pins are shared.

use std::path::PathBuf;

pub fn path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

fn normalize(uri: &str) -> String {
    uri.trim_end_matches('/').to_string()
}

pub fn read_from(path: &std::path::Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, ' ');
            let uri = parts.next()?.trim();
            if uri.is_empty() {
                return None;
            }
            let name = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(uri)
                .to_string();
            Some((name, uri.to_string()))
        })
        .collect()
}

pub fn read() -> Vec<(String, String)> {
    read_from(&path())
}

pub fn is_pinned(uri: &str) -> bool {
    let needle = normalize(uri);
    read().iter().any(|(_, u)| normalize(u) == needle)
}

/// Adds the pin (creates file and folders if missing). Idempotent.
pub fn pin_to(path: &std::path::Path, name: &str, uri: &str) -> Result<(), glib::Error> {
    let needle = normalize(uri);
    if read_from(path).iter().any(|(_, u)| normalize(u) == needle) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| glib::Error::new(gio::IOErrorEnum::Failed, &format!("Cannot pin: {e}")))?;
    }
    let mut text = std::fs::read_to_string(path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!("{needle} {name}\n"));
    std::fs::write(path, text)
        .map_err(|e| glib::Error::new(gio::IOErrorEnum::Failed, &format!("Cannot pin: {e}")))
}

pub fn pin(name: &str, uri: &str) -> Result<(), glib::Error> {
    pin_to(&path(), name, uri)
}

/// Removes the pin. Returns `true` if it was there.
pub fn unpin_from(path: &std::path::Path, uri: &str) -> Result<bool, glib::Error> {
    let needle = normalize(uri);
    let kept: Vec<String> = read_from(path)
        .into_iter()
        .filter(|(_, u)| normalize(u) != needle)
        .map(|(name, u)| format!("{u} {name}"))
        .collect();
    let existed = kept.len() != read_from(path).len();
    if !existed {
        return Ok(false);
    }
    let mut text = kept.join("\n");
    if !text.is_empty() {
        text.push('\n');
    }
    std::fs::write(path, text)
        .map(|_| true)
        .map_err(|e| glib::Error::new(gio::IOErrorEnum::Failed, &format!("Cannot unpin: {e}")))
}

pub fn unpin(uri: &str) -> Result<bool, glib::Error> {
    unpin_from(&path(), uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_unpin_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        assert!(!is_pinned_in(&file, "file:///tmp/docs"));

        pin_to(&file, "Docs", "file:///tmp/docs/").unwrap();
        pin_to(&file, "Docs", "file:///tmp/docs").unwrap(); // idempotent
        assert_eq!(read_from(&file).len(), 1);

        assert!(unpin_from(&file, "file:///tmp/docs").unwrap());
        assert!(!unpin_from(&file, "file:///tmp/docs").unwrap());
        assert!(read_from(&file).is_empty());
    }

    fn is_pinned_in(path: &std::path::Path, uri: &str) -> bool {
        let needle = normalize(uri);
        read_from(path).iter().any(|(_, u)| normalize(u) == needle)
    }
}
