use super::save_cache;
use crate::model::RecentItem;
use anyhow::{anyhow, Context, Result};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, RecvTimeoutError, SyncSender},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

const QUIET_PERIOD: Duration = Duration::from_millis(250);
const MAX_WRITE_DELAY: Duration = Duration::from_secs(1);

enum WriteRequest {
    Save,
    Flush(SyncSender<std::result::Result<(), String>>),
}

#[derive(Clone)]
pub struct CacheWriter {
    sender: SyncSender<WriteRequest>,
}

impl CacheWriter {
    pub fn start(
        items: Arc<RwLock<Vec<RecentItem>>>,
        cache_file: PathBuf,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);

        let _handle = std::thread::Builder::new()
            .name("fastaccess-cache-writer".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let mut flush_waiters = Vec::new();
                    let mut disconnected = false;

                    match request {
                        WriteRequest::Save => {
                            let deadline = Instant::now() + MAX_WRITE_DELAY;
                            loop {
                                let now = Instant::now();
                                if now >= deadline {
                                    break;
                                }
                                let wait =
                                    QUIET_PERIOD.min(deadline.saturating_duration_since(now));
                                match receiver.recv_timeout(wait) {
                                    Ok(WriteRequest::Save) => {}
                                    Ok(WriteRequest::Flush(waiter)) => {
                                        flush_waiters.push(waiter);
                                        break;
                                    }
                                    Err(RecvTimeoutError::Timeout) => break,
                                    Err(RecvTimeoutError::Disconnected) => {
                                        disconnected = true;
                                        break;
                                    }
                                }
                            }
                        }
                        WriteRequest::Flush(waiter) => {
                            flush_waiters.push(waiter);
                        }
                    }

                    let save_result = match items.read() {
                        Ok(items) => {
                            save_cache(&cache_file, items.as_slice())
                                .map_err(|error| error.to_string())
                        }
                        Err(error) => Err(format!("recent history lock is poisoned: {error}")),
                    };
                    for waiter in flush_waiters {
                        let result = match &save_result {
                            Ok(()) => Ok(()),
                            Err(error) => Err(error.clone()),
                        };
                        let _ = waiter.send(result);
                    }

                    if disconnected {
                        break;
                    }
                }
            })?;

        Ok(Self { sender })
    }

    pub fn request_save(&self) {
        let _ = self.sender.try_send(WriteRequest::Save);
    }

    pub fn flush(&self) -> Result<()> {
        let (waiter, completion) = mpsc::sync_channel(0);
        self.sender
            .send(WriteRequest::Flush(waiter))
            .context("cache writer is not running")?;
        completion
            .recv()
            .context("cache writer stopped before flushing")?
            .map_err(|error| anyhow!(error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        cache::load_cache,
        model::{ItemKind, RecentItem},
    };

    fn item(path: &str, observed_at_ms: u64) -> RecentItem {
        RecentItem::new(PathBuf::from(path), observed_at_ms, ItemKind::File)
    }

    fn test_cache_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "fastaccess-writer-test-{}-{name}",
                std::process::id()
            ))
            .join("cache.json")
    }

    #[test]
    fn flush_persists_the_latest_snapshot() {
        let path = test_cache_path("flush");
        let items = Arc::new(RwLock::new(vec![item("old.txt", 1)]));
        let writer = CacheWriter::start(Arc::clone(&items), path.clone()).unwrap();

        writer.request_save();
        items.write().unwrap().insert(0, item("new.txt", 2));
        writer.request_save();
        writer.flush().unwrap();

        let cache = load_cache(&path).unwrap();
        assert_eq!(cache.items[0].display_name, "new.txt");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn poisoned_history_lock_does_not_replace_an_existing_cache() {
        let path = test_cache_path("poison");
        let original = vec![item("keep.txt", 1)];
        save_cache(&path, &original).unwrap();

        let items = Arc::new(RwLock::new(vec![item("discard.txt", 2)]));
        let poisoned = Arc::clone(&items);
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned.write().unwrap();
            panic!("poison test lock");
        });

        let writer = CacheWriter::start(items, path.clone()).unwrap();
        assert!(writer.flush().is_err());

        let cache = load_cache(&path).unwrap();
        assert_eq!(cache.items, original);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
