use super::{known_folder::recent_folder, shortcut::parse_shortcut};
use crate::model::RecentItem;
use anyhow::{Context, Result};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default)]
pub struct ScanReport {
    pub items: Vec<RecentItem>,
    pub shortcuts_seen: usize,
    pub shortcuts_skipped: usize,
}

pub fn scan_recent() -> Result<ScanReport> {
    scan_directory(&recent_folder()?)
}

pub fn scan_directory(directory: &Path) -> Result<ScanReport> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("cannot read Recent Items folder {}", directory.display()))?;
    let mut candidates = Vec::new();
    let mut report = ScanReport::default();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.shortcuts_skipped += 1;
                continue;
            }
        };
        let path = entry.path();
        if !is_lnk(&path) {
            continue;
        }
        report.shortcuts_seen += 1;
        let observed_at_ms = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(system_time_ms)
            .unwrap_or(0);
        candidates.push((path, observed_at_ms));
    }

    candidates.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    let mut unique_targets = HashSet::new();

    for (path, observed_at_ms) in candidates {
        match parse_shortcut(&path, observed_at_ms) {
            Ok(item) => {
                let key = item.target.to_string_lossy().to_lowercase();
                if unique_targets.insert(key) {
                    report.items.push(item);
                }
            }
            Err(_) => report.shortcuts_skipped += 1,
        }
    }

    Ok(report)
}

fn is_lnk(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
