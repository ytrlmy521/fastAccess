use crate::model::RecentItem;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, File},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

// Schema 1 may contain paths decoded with the wrong Windows ANSI code page.
// The original bytes are unavailable, so rebuilding from Windows Recent is the
// only lossless migration.
pub const CACHE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheFile {
    pub schema_version: u32,
    pub generated_at: u64,
    pub items: Vec<RecentItem>,
}

impl CacheFile {
    pub fn new(items: Vec<RecentItem>) -> Self {
        Self {
            schema_version: CACHE_SCHEMA_VERSION,
            generated_at: now_ms(),
            items,
        }
    }
}

pub fn cache_path() -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is not defined")?;
        Ok(PathBuf::from(base).join("FastAccess").join("cache.json"))
    }

    #[cfg(not(windows))]
    {
        let base = env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .context("neither XDG_CACHE_HOME nor HOME is defined")?;
        Ok(base.join("fastaccess").join("cache.json"))
    }
}

pub fn load_cache(path: &Path) -> Result<CacheFile> {
    let file = File::open(path).with_context(|| format!("cannot open cache {}", path.display()))?;
    let cache: CacheFile = serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("cannot decode cache {}", path.display()))?;
    if cache.schema_version != CACHE_SCHEMA_VERSION {
        bail!(
            "unsupported cache schema {}, expected {}",
            cache.schema_version,
            CACHE_SCHEMA_VERSION
        );
    }
    Ok(cache)
}

pub fn save_cache(path: &Path, items: &[RecentItem]) -> Result<()> {
    let directory = path
        .parent()
        .with_context(|| format!("cache path has no parent: {}", path.display()))?;
    fs::create_dir_all(directory)
        .with_context(|| format!("cannot create cache directory {}", directory.display()))?;

    let temporary = path.with_extension("json.tmp");
    {
        let file = File::create(&temporary)
            .with_context(|| format!("cannot create temporary cache {}", temporary.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer(&mut writer, &CacheFile::new(items.to_vec()))
            .context("cannot encode cache")?;
        writer.flush().context("cannot flush cache buffer")?;
        writer
            .get_ref()
            .sync_all()
            .context("cannot sync temporary cache")?;
    }

    atomic_replace(&temporary, path)?;
    sync_directory_best_effort(directory);
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot replace cache atomically");
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).with_context(|| {
        format!(
            "cannot replace cache {} with {}",
            destination.display(),
            source.display()
        )
    })
}

#[cfg(unix)]
fn sync_directory_best_effort(directory: &Path) {
    if let Ok(file) = File::open(directory) {
        let _ = file.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory_best_effort(_directory: &Path) {}

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
    use crate::model::ItemKind;

    #[test]
    fn cache_round_trip() {
        let directory = std::env::temp_dir().join(format!(
            "fastaccess-cache-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let path = directory.join("cache.json");
        let items = vec![RecentItem::new(
            PathBuf::from("example.txt"),
            123,
            ItemKind::File,
        )];

        save_cache(&path, &items).unwrap();
        let loaded = load_cache(&path).unwrap();
        assert_eq!(loaded.schema_version, CACHE_SCHEMA_VERSION);
        assert_eq!(loaded.items, items);

        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(directory);
    }

    #[test]
    fn rejects_legacy_cache_with_ambiguous_path_encoding() {
        let directory = std::env::temp_dir().join(format!(
            "fastaccess-cache-test-legacy-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let path = directory.join("cache.json");
        fs::create_dir_all(&directory).unwrap();
        let legacy = CacheFile {
            schema_version: 1,
            generated_at: 123,
            items: vec![RecentItem::new(
                PathBuf::from(r"E:\ÖÇÓ°µ¼³ö"),
                123,
                ItemKind::Folder,
            )],
        };
        serde_json::to_writer(File::create(&path).unwrap(), &legacy).unwrap();

        let error = load_cache(&path).unwrap_err().to_string();

        assert!(error.contains("unsupported cache schema 1"));
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(directory);
    }
}
