#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use fastaccess::{
    cache::{cache_path, load_cache, CacheWriter},
    model::{ItemKind, RecentItem},
    platform::{open_target, start_explorer_tracker, start_hotkey_listener},
    recent::{mark_accessed, merge_recent_items, scan_recent, MAX_RECENT_ITEMS},
    search::search,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

slint::include_modules!();

const RESULT_LIMIT: usize = 50;
const APP_TITLE: &str = "FastAccess";

fn main() -> Result<()> {
    let app = AppWindow::new().context("cannot create FastAccess window")?;
    let cache_file = cache_path().context("cannot determine cache path")?;
    let mut cached_items = load_cache(&cache_file)
        .map(|cache| cache.items)
        .unwrap_or_default();
    cached_items.sort_unstable_by_key(|item| Reverse(item.observed_at_ms));
    cached_items.truncate(MAX_RECENT_ITEMS);
    let items = Arc::new(RwLock::new(cached_items));
    let cache_writer =
        CacheWriter::start(Arc::clone(&items), cache_file).context("cannot start cache writer")?;

    let launch_weak = app.as_weak();
    let target_launcher = TargetLauncher::start(move |target, result| {
        let Err(error) = result else {
            return;
        };
        let message = format!("Cannot open {}: {error:#}", target.display());
        let weak = launch_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                app.set_status_text(message.into());
                let _ = show_and_activate(&app);
            }
        });
    })
    .context("cannot start target launcher")?;

    install_ui_callbacks(
        &app,
        Arc::clone(&items),
        cache_writer.clone(),
        target_launcher,
    );
    let explorer_items = Arc::clone(&items);
    let explorer_writer = cache_writer.clone();
    let explorer_weak = app.as_weak();
    let explorer_tracker = start_explorer_tracker(move |target| {
        let changed = explorer_items
            .write()
            .map(|mut items| mark_accessed(&mut items, target, ItemKind::Folder))
            .unwrap_or(false);
        if !changed {
            return;
        }

        explorer_writer.request_save();
        let ui_items = Arc::clone(&explorer_items);
        let weak = explorer_weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                if app.window().is_visible() {
                    let query = app.get_query();
                    render_query(&app, &ui_items, query.as_str());
                }
            }
        });
    })
    .context("cannot monitor Explorer folder navigation")?;
    app.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    render_query(&app, &items, "");
    let recent_collector =
        RecentCollector::start(app.as_weak(), Arc::clone(&items), cache_writer.clone())
            .context("cannot start Windows Recent collector")?;
    recent_collector.request_refresh();
    start_hotkey(
        app.as_weak(),
        Arc::clone(&items),
        recent_collector.requester(),
    )?;

    show_and_activate(&app)?;
    slint::run_event_loop_until_quit().context("Slint event loop failed")?;
    drop(explorer_tracker);
    drop(recent_collector);
    cache_writer
        .flush()
        .context("cannot flush recent history cache")?;
    Ok(())
}

fn install_ui_callbacks(
    app: &AppWindow,
    items: Arc<RwLock<Vec<RecentItem>>>,
    cache_writer: CacheWriter,
    target_launcher: TargetLauncher,
) {
    let weak = app.as_weak();
    let search_items = Arc::clone(&items);
    app.on_search_changed(move |query| {
        if let Some(app) = weak.upgrade() {
            render_query(&app, &search_items, query.as_str());
        }
    });

    let weak = app.as_weak();
    let accessed_items = Arc::clone(&items);
    app.on_open_requested(move |index| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        if index < 0 {
            return;
        }
        let results = app.get_results();
        let Some(item) = results.row_data(index as usize) else {
            return;
        };
        let target = PathBuf::from(item.path.as_str());
        if let Err(error) = target_launcher.try_open(target.clone()) {
            app.set_status_text(format!("Cannot open item: {error:#}").into());
            return;
        }
        let kind = if item.kind.as_str() == "Folder" {
            ItemKind::Folder
        } else {
            ItemKind::File
        };
        let changed = accessed_items
            .write()
            .map(|mut current| mark_accessed(&mut current, target, kind))
            .unwrap_or(false);
        let query = app.get_query();
        render_query(&app, &accessed_items, query.as_str());
        if changed {
            cache_writer.request_save();
        }
        let _ = app.hide();
    });

    let weak = app.as_weak();
    app.on_dismiss_requested(move || {
        if let Some(app) = weak.upgrade() {
            let _ = app.hide();
        }
    });

    app.on_quit_requested(|| {
        let _ = slint::quit_event_loop();
    });
}

fn render_query(app: &AppWindow, items: &RwLock<Vec<RecentItem>>, query: &str) {
    let matches = items
        .read()
        .map(|items| search(&items, query, RESULT_LIMIT))
        .unwrap_or_default();
    let result_count = matches.len();
    let rows: Vec<UiRecentItem> = matches.iter().map(to_ui_item).collect();

    app.set_results(ModelRc::new(VecModel::from(rows)));
    app.set_selected_index(if result_count > 0 { 0 } else { -1 });
    app.set_status_text(
        if query.trim().is_empty() {
            "No recently accessed items yet"
        } else {
            "No matching recent items"
        }
        .into(),
    );
}

fn to_ui_item(item: &RecentItem) -> UiRecentItem {
    UiRecentItem {
        name: SharedString::from(item.display_name.as_str()),
        path: SharedString::from(item.display_path.as_str()),
        accessed: SharedString::from(format_relative_time(item.observed_at_ms)),
        kind: SharedString::from(match item.kind {
            ItemKind::File => "File",
            ItemKind::Folder => "Folder",
        }),
    }
}

enum RecentRequest {
    Refresh,
    Stop,
}

struct RecentCollector {
    sender: mpsc::SyncSender<RecentRequest>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct RecentCollectorRequester {
    sender: mpsc::SyncSender<RecentRequest>,
}

impl RecentCollector {
    fn start(
        weak: slint::Weak<AppWindow>,
        items: Arc<RwLock<Vec<RecentItem>>>,
        cache_writer: CacheWriter,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = std::thread::Builder::new()
            .name("fastaccess-recent-collector".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if matches!(request, RecentRequest::Stop) {
                        break;
                    }

                    match scan_recent() {
                        Ok(report) => {
                            let changed = items
                                .write()
                                .map(|mut current| {
                                    let merged =
                                        merge_recent_items(current.as_slice(), report.items);
                                    if *current == merged {
                                        false
                                    } else {
                                        *current = merged;
                                        true
                                    }
                                })
                                .unwrap_or(false);
                            if changed {
                                cache_writer.request_save();
                            }
                            let ui_items = Arc::clone(&items);
                            let ui_weak = weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = ui_weak.upgrade() {
                                    let query = app.get_query();
                                    render_query(&app, &ui_items, query.as_str());
                                }
                            });
                        }
                        Err(error) => {
                            let message = format!("Recent scan failed: {error:#}");
                            let ui_weak = weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(app) = ui_weak.upgrade() {
                                    app.set_status_text(message.into());
                                }
                            });
                        }
                    }
                }
            })?;
        Ok(Self {
            sender,
            handle: Some(handle),
        })
    }

    fn request_refresh(&self) {
        let _ = self.sender.try_send(RecentRequest::Refresh);
    }

    fn requester(&self) -> RecentCollectorRequester {
        RecentCollectorRequester {
            sender: self.sender.clone(),
        }
    }
}

impl Drop for RecentCollector {
    fn drop(&mut self) {
        let _ = self.sender.send(RecentRequest::Stop);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl RecentCollectorRequester {
    fn request_refresh(&self) {
        let _ = self.sender.try_send(RecentRequest::Refresh);
    }
}

fn start_hotkey(
    weak: slint::Weak<AppWindow>,
    items: Arc<RwLock<Vec<RecentItem>>>,
    recent_collector: RecentCollectorRequester,
) -> Result<()> {
    let listener = start_hotkey_listener(move || {
        let weak = weak.clone();
        let items = Arc::clone(&items);
        let recent_collector = recent_collector.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                if app.window().is_visible() && is_fastaccess_foreground() {
                    let _ = app.hide();
                } else {
                    app.set_query(SharedString::default());
                    render_query(&app, &items, "");
                    if let Err(error) = show_and_activate(&app) {
                        app.set_status_text(
                            format!("Cannot activate FastAccess: {error:#}").into(),
                        );
                    }
                    recent_collector.request_refresh();
                }
            }
        });
    })?;

    // The listener owns its thread and lives until process shutdown.
    drop(listener);
    Ok(())
}

#[derive(Clone)]
struct TargetLauncher {
    sender: mpsc::SyncSender<PathBuf>,
    busy: Arc<AtomicBool>,
}

impl TargetLauncher {
    fn start<F>(on_complete: F) -> std::io::Result<Self>
    where
        F: Fn(PathBuf, Result<()>) + Send + 'static,
    {
        Self::start_with(open_target, on_complete)
    }

    fn start_with<O, F>(open: O, on_complete: F) -> std::io::Result<Self>
    where
        O: Fn(&Path) -> Result<()> + Send + 'static,
        F: Fn(PathBuf, Result<()>) + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = Arc::clone(&busy);
        let _handle = std::thread::Builder::new()
            .name("fastaccess-target-launcher".into())
            .spawn(move || {
                while let Ok(target) = receiver.recv() {
                    let result = open(&target);
                    worker_busy.store(false, Ordering::Release);
                    on_complete(target, result);
                }
            })?;
        Ok(Self { sender, busy })
    }

    fn try_open(&self, target: PathBuf) -> Result<()> {
        if self.busy.swap(true, Ordering::AcqRel) {
            return Err(anyhow::anyhow!("another item is still opening"));
        }

        match self.sender.try_send(target) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => {
                self.busy.store(false, Ordering::Release);
                Err(anyhow::anyhow!("target launcher is busy"))
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.busy.store(false, Ordering::Release);
                Err(anyhow::anyhow!("target launcher is not running"))
            }
        }
    }
}

fn show_and_activate(app: &AppWindow) -> Result<()> {
    app.show().context("cannot show FastAccess window")?;
    activate_fastaccess_window();
    Ok(())
}

#[cfg(windows)]
fn activate_fastaccess_window() {
    use std::ptr;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOMOVE,
        SWP_NOSIZE, SWP_SHOWWINDOW, SW_RESTORE,
    };

    let title: Vec<u16> = APP_TITLE.encode_utf16().chain(Some(0)).collect();
    let window = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
    if window.is_null() {
        return;
    }

    unsafe {
        ShowWindow(window, SW_RESTORE);
        SetWindowPos(
            window,
            HWND_TOP,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
        );
        SetForegroundWindow(window);
    }
}

#[cfg(not(windows))]
fn activate_fastaccess_window() {}

#[cfg(windows)]
fn is_fastaccess_foreground() -> bool {
    use std::ptr;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetForegroundWindow};

    let title: Vec<u16> = APP_TITLE.encode_utf16().chain(Some(0)).collect();
    let window = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
    !window.is_null() && unsafe { GetForegroundWindow() == window }
}

#[cfg(not(windows))]
fn is_fastaccess_foreground() -> bool {
    true
}

fn format_relative_time(observed_at_ms: u64) -> String {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let elapsed_seconds = now_ms.saturating_sub(observed_at_ms) / 1_000;

    match elapsed_seconds {
        0..=59 => "Just now".into(),
        60..=3_599 => format!("{} min ago", elapsed_seconds / 60),
        3_600..=86_399 => format!("{} hr ago", elapsed_seconds / 3_600),
        86_400..=604_799 => format!("{} d ago", elapsed_seconds / 86_400),
        _ => format!("{} wk ago", elapsed_seconds / 604_800),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn target_launcher_keeps_slow_shell_work_off_the_calling_thread() {
        let (worker_tx, worker_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let (completed_tx, completed_rx) = mpsc::sync_channel(0);

        let launcher = TargetLauncher::start_with(
            move |_target| {
                worker_tx.send(std::thread::current().id()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            },
            move |_target, result| {
                completed_tx.send(result.is_ok()).unwrap();
            },
        )
        .unwrap();
        let (request_tx, request_rx) = mpsc::sync_channel(0);
        let request_launcher = launcher.clone();
        let caller = std::thread::spawn(move || {
            let result = request_launcher.try_open(PathBuf::from("slow-target"));
            request_tx
                .send((std::thread::current().id(), result.is_ok()))
                .unwrap();
        });

        let worker_thread = worker_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let (caller_thread, request_succeeded) =
            request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request_succeeded);
        assert_ne!(worker_thread, caller_thread);
        assert!(launcher
            .try_open(PathBuf::from("second-target"))
            .unwrap_err()
            .to_string()
            .contains("still opening"));

        release_tx.send(()).unwrap();
        assert!(completed_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        caller.join().unwrap();
    }
}
