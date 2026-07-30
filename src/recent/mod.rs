mod history;
mod known_folder;
mod scanner;
mod shortcut;

pub use history::{mark_accessed, merge_recent_items, MAX_RECENT_ITEMS};
pub use scanner::{scan_recent, ScanReport};
