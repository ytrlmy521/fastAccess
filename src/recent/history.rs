use crate::model::{ItemKind, RecentItem};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_RECENT_ITEMS: usize = 500;
const DUPLICATE_EVENT_WINDOW_MS: u64 = 1_500;

pub fn mark_accessed(
    items: &mut Vec<RecentItem>,
    target: PathBuf,
    kind: ItemKind,
) -> bool {
    mark_accessed_at(items, target, kind, now_ms())
}

pub fn merge_recent_items(
    existing: &[RecentItem],
    discovered: Vec<RecentItem>,
) -> Vec<RecentItem> {
    let mut by_target = HashMap::<String, RecentItem>::new();

    for item in existing.iter().cloned().chain(discovered) {
        let key = target_key(&item.target);
        match by_target.get_mut(&key) {
            Some(current) if item.observed_at_ms > current.observed_at_ms => *current = item,
            Some(_) => {}
            None => {
                by_target.insert(key, item);
            }
        }
    }

    let mut merged: Vec<_> = by_target.into_values().collect();
    merged.sort_unstable_by(|a, b| {
        b.observed_at_ms
            .cmp(&a.observed_at_ms)
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.display_path.cmp(&b.display_path))
    });
    merged.truncate(MAX_RECENT_ITEMS);
    merged
}

fn mark_accessed_at(
    items: &mut Vec<RecentItem>,
    target: PathBuf,
    kind: ItemKind,
    observed_at_ms: u64,
) -> bool {
    let key = target_key(&target);
    if items.first().is_some_and(|item| {
        target_key(&item.target) == key
            && observed_at_ms.saturating_sub(item.observed_at_ms) <= DUPLICATE_EVENT_WINDOW_MS
    }) {
        return false;
    }

    items.retain(|item| target_key(&item.target) != key);
    items.insert(0, RecentItem::new(target, observed_at_ms, kind));
    items.truncate(MAX_RECENT_ITEMS);
    true
}

fn target_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, observed_at_ms: u64) -> RecentItem {
        RecentItem::new(PathBuf::from(path), observed_at_ms, ItemKind::File)
    }

    #[test]
    fn accessed_item_moves_to_the_front_without_duplicates() {
        let mut items = vec![item("new.txt", 20), item("old.txt", 10)];

        let changed = mark_accessed_at(
            &mut items,
            PathBuf::from("old.txt"),
            ItemKind::File,
            30,
        );

        assert!(changed);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].display_name, "old.txt");
        assert_eq!(items[0].observed_at_ms, 30);
    }

    #[test]
    fn duplicate_navigation_event_is_ignored() {
        let mut items = vec![item("folder", 10_000)];

        let changed = mark_accessed_at(
            &mut items,
            PathBuf::from("folder"),
            ItemKind::Folder,
            10_500,
        );

        assert!(!changed);
        assert_eq!(items[0].observed_at_ms, 10_000);
    }

    #[test]
    fn history_has_a_fixed_capacity() {
        let mut items: Vec<_> = (0..MAX_RECENT_ITEMS)
            .map(|index| item(&format!("{index}.txt"), index as u64))
            .collect();

        mark_accessed_at(
            &mut items,
            PathBuf::from("new.txt"),
            ItemKind::File,
            10_000,
        );

        assert_eq!(items.len(), MAX_RECENT_ITEMS);
        assert_eq!(items[0].display_name, "new.txt");
    }

    #[test]
    fn merge_keeps_the_newest_observation_for_each_target() {
        let existing = vec![item(r"C:\Work\report.docx", 30), item("cached.txt", 20)];
        let discovered = vec![
            item(r"c:\work\REPORT.docx", 10),
            item("system.txt", 40),
        ];

        let merged = merge_recent_items(&existing, discovered);

        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].display_name, "system.txt");
        let report = merged
            .iter()
            .find(|item| item.display_name == "report.docx")
            .unwrap();
        assert_eq!(report.observed_at_ms, 30);
    }
}
