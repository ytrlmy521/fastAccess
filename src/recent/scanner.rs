use super::{known_folder::recent_folder, shortcut::parse_shortcut};
use crate::model::{ItemKind, RecentItem};
use anyhow::Result;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Default)]
pub struct ScanReport {
    pub items: Vec<RecentItem>,
    pub shortcuts_seen: usize,
    pub shortcuts_skipped: usize,
}

const RECENT_DAYS: u64 = 30;

pub fn scan_recent() -> Result<ScanReport> {
    let mut report = ScanReport::default();
    let mut seen = HashSet::new();

    if let Ok(folder) = recent_folder() {
        scan_lnk_directory(&folder, &mut report, &mut seen);
    }

    let additional = known_user_dirs();
    for dir in additional {
        scan_directory_for_recent(&dir, &mut report, &mut seen);
    }

    report.items.sort_unstable_by(|a, b| b.observed_at_ms.cmp(&a.observed_at_ms));
    Ok(report)
}

fn scan_lnk_directory(directory: &Path, report: &mut ScanReport, seen: &mut HashSet<String>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };

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
}

fn scan_directory_for_recent(dir: &Path, report: &mut ScanReport, seen: &mut HashSet<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(RECENT_DAYS * 86400))
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    for entry in entries.flatten() {
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let modified = metadata.modified().map(system_time_ms).unwrap_or(0);
        if modified < cutoff {
            continue;
        }

        let is_dir = metadata.is_dir();
        let target_key = path.to_string_lossy().to_lowercase();
        if seen.insert(target_key) {
            let kind = if is_dir { ItemKind::Folder } else { ItemKind::File };
            report.items.push(RecentItem::new(path, modified, kind));
        }
    }
}

fn known_user_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE") {
        let base = PathBuf::from(&home);
        for sub in &["Desktop", "Downloads", "Documents"] {
            dirs.push(base.join(sub));
        }
        dirs.push(base.join("Recent").join("AutomaticDestinations"));
        let recent = base.join("AppData").join("Roaming").join("Microsoft").join("Windows").join("Recent");
        if recent.exists() {
            dirs.push(recent);
        }
        dirs.push(base.join("AppData").join("Roaming").join("Microsoft").join("Windows").join("Recent").join("AutomaticDestinations"));
    }
    dirs.retain(|d| d.exists());
    dirs
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
