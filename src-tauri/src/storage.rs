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
