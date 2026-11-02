//! Shared helpers for the app's files (graphs, cables, settings).

use std::fs;
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub type CmdResult<T> = Result<T, String>;

pub fn err(e: impl ToString) -> String {
    e.to_string()
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Writes through a temporary file: if writing fails midway, the old file stays intact
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

/// ids go into file names, so only safe characters pass
pub fn check_id(id: &str) -> CmdResult<()> {
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(tr!("error_invalid_id", id = id))
    }
}

/// "Name", or if taken "Name 2", "Name 3"... (as uniqueName in src/graph/names.ts)
pub fn unique_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: std::collections::HashSet<&str> = taken.into_iter().collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken.contains(n.as_str())).unwrap_or_default()
}

pub fn clean_name(name: &str) -> CmdResult<String> {
    let name = name.trim();
    if name.is_empty() {
        Err(tr!("error_empty_name"))
    } else {
        Ok(name.chars().take(100).collect())
    }
}

/// An empty folder of the test's own for its files
#[cfg(test)]
pub fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("audio-graph-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_safe_file_names() {
        assert!(check_id("51d05ea4-b887-4599-a6b9-1161a689abff").is_ok());
        for bad in ["", "../preferences", "a/b", r"a\b", "a.json", "c:", "имя", &"a".repeat(65)] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn names_are_trimmed_and_cut_by_characters() {
        assert_eq!(clean_name("  Music \n").unwrap(), "Music");
        assert!(clean_name(" \t ").is_err());
        // a cut in the middle of a letter would be invalid text
        let name = clean_name(&"я".repeat(150)).unwrap();
        assert_eq!(name.chars().count(), 100);
    }

    #[test]
    fn unique_names_count_up() {
        assert_eq!(unique_name("Music", ["Games"]), "Music");
        assert_eq!(unique_name("Music", ["Music", "Music 2"]), "Music 3");
        // a gap is taken first
        assert_eq!(unique_name("Music", ["Music", "Music 3"]), "Music 2");
    }

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_nothing_behind() {
        let dir = test_dir("atomic");
        let path = dir.join("cables.json");
        write_atomic(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, ["cables.json"]);
    }
}
