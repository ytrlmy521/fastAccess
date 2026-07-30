use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentItem {
    /// Filesystem target observed from Explorer or resolved from a Recent shortcut.
    pub target: PathBuf,
    /// Filename or final folder component used by the UI.
    pub display_name: String,
    /// Full path shown by the UI.
    pub display_path: String,
    /// Precomputed, normalized text used only by the in-memory matcher.
    pub search_text: String,
    /// Observation timestamp used for ordering. This is not NTFS last-access time.
    pub observed_at_ms: u64,
    pub kind: ItemKind,
}

impl RecentItem {
    pub fn new(target: PathBuf, observed_at_ms: u64, kind: ItemKind) -> Self {
        let display_name = display_name(&target);
        let display_path = target.to_string_lossy().into_owned();
        let search_text = format!("{display_name} {display_path}").to_lowercase();

        Self {
            target,
            display_name,
            display_path,
            search_text,
            observed_at_ms,
            kind,
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_searchable_item() {
        let item = RecentItem::new(
            PathBuf::from(r"C:\Work\Quarterly Report.docx"),
            42,
            ItemKind::File,
        );

        assert_eq!(item.display_name, "Quarterly Report.docx");
        assert!(item.search_text.contains("quarterly report.docx"));
        assert!(item.search_text.contains(r"c:\work"));
    }
}
