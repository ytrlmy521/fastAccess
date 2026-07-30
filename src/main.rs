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
    path::PathBuf,
    sync::{mpsc, Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

slint::include_modules!();

const RESULT_LIMIT: usize = 50;

fn main() -> Result<()> {
    let app = AppWindow::new().context("cannot create FastAccess window")?;
    let cache_file = cache_path().context("cannot determine cache path")?;
    let mut cached_items = load_cache(&cache_file)
        .map(|cache| cache.items)
        .unwrap_or_default();
    cached_items.sort_unstable_by(|a, b| b.observed_at_ms.cmp(&a.observed_at_ms));
    cached_items.truncate(MAX_RECENT_ITEMS);
    let items = Arc::new(RwLock::new(cached_items));
    let cache_writer = CacheWriter::start(Arc::clone(&items), cache_file)
        .context("cannot start cache writer")?;

    install_ui_callbacks(&app, Arc::clone(&items), cache_writer.clone());
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
    let recent_collector = RecentCollector::start(
        app.as_weak(),
        Arc::clone(&items),
        cache_writer.clone(),
    )
    .context("cannot start Windows Recent collector")?;
    recent_collector.request_refresh();
    start_hotkey(
        app.as_weak(),
        Arc::clone(&items),
        recent_collector.requester(),
    )?;

    app.show().context("cannot show FastAccess window")?;
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
        if let Err(error) = open_target(&target) {
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
                if app.window().is_visible() {
                    let _ = app.hide();
                } else {
                    app.set_query(SharedString::default());
                    render_query(&app, &items, "");
                    let _ = app.show();
                    recent_collector.request_refresh();
                }
            }
        });
    })?;

    // The listener owns its thread and lives until process shutdown.
    drop(listener);
    Ok(())
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
