#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use anyhow::{Context, Result};
use fastaccess::{
    cache::{cache_path, load_cache, save_cache},
    model::{ItemKind, RecentItem},
    platform::{open_target, start_hotkey_listener},
    recent::scan_recent,
    search::search,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    path::PathBuf,
    sync::{Arc, RwLock},
};

slint::include_modules!();

const RESULT_LIMIT: usize = 50;

fn main() -> Result<()> {
    let app = AppWindow::new().context("cannot create FastAccess window")?;
    let cache_file = cache_path().context("cannot determine cache path")?;
    let cached_items = load_cache(&cache_file)
        .map(|cache| cache.items)
        .unwrap_or_default();
    let items = Arc::new(RwLock::new(cached_items));

    install_ui_callbacks(&app, Arc::clone(&items));
    app.window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    render_query(&app, &items, "");
    start_collector(app.as_weak(), Arc::clone(&items), cache_file);
    start_hotkey(app.as_weak())?;

    app.show().context("cannot show FastAccess window")?;
    slint::run_event_loop_until_quit().context("Slint event loop failed")?;
    Ok(())
}

fn install_ui_callbacks(app: &AppWindow, items: Arc<RwLock<Vec<RecentItem>>>) {
    let weak = app.as_weak();
    let search_items = Arc::clone(&items);
    app.on_search_changed(move |query| {
        if let Some(app) = weak.upgrade() {
            render_query(&app, &search_items, query.as_str());
        }
    });

    let weak = app.as_weak();
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
            "No Windows Recent items found"
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
        kind: SharedString::from(match item.kind {
            ItemKind::File => "File",
            ItemKind::Folder => "Folder",
        }),
    }
}

fn start_collector(
    weak: slint::Weak<AppWindow>,
    items: Arc<RwLock<Vec<RecentItem>>>,
    cache_file: PathBuf,
) {
    std::thread::Builder::new()
        .name("fastaccess-collector".into())
        .spawn(move || match scan_recent() {
            Ok(report) => {
                let snapshot = report.items;
                if let Ok(mut current) = items.write() {
                    *current = snapshot.clone();
                }
                let _ = save_cache(&cache_file, &snapshot);
                let ui_items = Arc::clone(&items);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        let query = app.get_query();
                        render_query(&app, &ui_items, query.as_str());
                    }
                });
            }
            Err(error) => {
                let message = format!("Recent scan failed: {error:#}");
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(app) = weak.upgrade() {
                        app.set_status_text(message.into());
                    }
                });
            }
        })
        .expect("cannot start collector thread");
}

fn start_hotkey(weak: slint::Weak<AppWindow>) -> Result<()> {
    let listener = start_hotkey_listener(move || {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = weak.upgrade() {
                if app.window().is_visible() {
                    let _ = app.hide();
                } else {
                    app.set_query(SharedString::default());
                    app.set_selected_index(if app.get_results().row_count() > 0 {
                        0
                    } else {
                        -1
                    });
                    let _ = app.show();
                }
            }
        });
    })?;

    // The listener owns its thread and lives until process shutdown.
    drop(listener);
    Ok(())
}
