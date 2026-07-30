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
    let mut report = ScanReport::default();
    let mut seen = HashSet::new();

    let folder = recent_folder()?;
    scan_lnk_directory(&folder, &mut report, &mut seen)?;

    report.items.sort_unstable_by(|a, b| b.observed_at_ms.cmp(&a.observed_at_ms));
    Ok(report)
}

fn scan_lnk_directory(
    directory: &Path,
    report: &mut ScanReport,
    seen: &mut HashSet<String>,
) -> Result<()> {
    let entries = fs::read_dir(directory)
        .with_context(|| format!("cannot read Windows Recent directory {}", directory.display()))?;

    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_lnk(&path) {
            continue;
        }
        report.shortcuts_seen += 1;
        let observed_at_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .map(system_time_ms)
            .unwrap_or(0);
        candidates.push((path, observed_at_ms));
    }

    candidates.sort_unstable_by(|a, b| b.1.cmp(&a.1));
    for (path, observed_at_ms) in candidates {
        match parse_shortcut(&path, observed_at_ms) {
            Ok(item) => {
                let key = item.target.to_string_lossy().to_lowercase();
                if seen.insert(key) {
                    report.items.push(item);
                }
            }
            Err(_) => report.shortcuts_skipped += 1,
        }
    }
    Ok(())
}

fn is_lnk(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
