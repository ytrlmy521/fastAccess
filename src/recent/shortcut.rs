use crate::model::{ItemKind, RecentItem};
use anyhow::{Context, Result};
use lnk::{encoding::WINDOWS_1252, FileAttributeFlags, ShellLink};
use std::path::{Path, PathBuf};

pub fn parse_shortcut(path: &Path, observed_at_ms: u64) -> Result<RecentItem> {
    let shortcut = ShellLink::open(path, WINDOWS_1252)
        .with_context(|| format!("cannot parse shortcut {}", path.display()))?;
    let target = shortcut
        .link_target()
        .filter(|target| !target.trim().is_empty())
        .map(PathBuf::from)
        .with_context(|| format!("shortcut has no filesystem target: {}", path.display()))?;

    let is_directory = shortcut
        .header()
        .file_attributes()
        .contains(FileAttributeFlags::FILE_ATTRIBUTE_DIRECTORY);
    let kind = if is_directory {
        ItemKind::Folder
    } else {
        ItemKind::File
    };

    Ok(RecentItem::new(target, observed_at_ms, kind))
}
